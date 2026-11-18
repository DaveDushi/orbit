/**
 * Jobs: one approval for a whole piece of work, such as an import of 2,000 customers into a shop, instead of one "yes"
 * per request. Orbit keeps and checks everything (`apps/server/src/agent/jobs.rs`):
 *
 * - `plan_job`: the model gives the plan (what, the one tool it writes with, the most calls, the field mapping, the
 *   columns left out, a few samples, the source file) and the script. Orbit stores it and builds the question from
 *   it; the driver answers yes once.
 * - `run_job`: after the yes, Orbit hands back the stored script (never code from the model) and the job runs for at
 *   most 60 minutes in code mode, with `records()` (the source file's rows) and `progress(done, total)`. Each write of
 *   the script names the job and the hash of the code that runs ([`jobRef`]): Orbit runs it without a new yes only
 *   inside the job's scope, and refuses a write of a job that ended, ran out, expired or was stopped ("stop"). The
 *   run's tool-call limit does not count the script's calls; the job's own most calls and its deadline do.
 * - The thread shows the progress ("870/2,224"); the first error stops the job, and the model gets a report of what was
 *   done, what failed and what is left.
 */

import type { Context } from "@earendil-works/chord/context";
import { Type } from "@earendil-works/pi-ai";
import { defineTool } from "@earendil-works/pi-durable";
import type { ConversationId, ToolExecutionApi, ToolExecutionResult } from "@earendil-works/pi-durable";
import type { CodemodeGlobal } from "@earendil-works/pi-durable/codemode";

import { codemodeWith } from "./codemode.ts";
import type { ThreadState } from "./docs.ts";
import { untrusted, type FileContent } from "./files.ts";
import { askForApproval, type As, type Driver } from "./integrations.ts";
import type { OrbitClient } from "./orbit.ts";

export const PLAN_JOB_TOOL = "plan_job";
export const RUN_JOB_TOOL = "run_job";
/** A job's deadline in the sandbox: Orbit ends it 60 minutes after the approval too. */
const JOB_TIMEOUT_MS = 60 * 60 * 1000;
/** Rows of a source file a job reads at most, and per request. */
export const MAX_RECORDS = 20_000;
const ROWS_PER_READ = 1_000;
/** Progress edits of the thread's job message, at most one per this many milliseconds. */
const PROGRESS_EVERY_MS = 2_000;

type Source = { kind: "chat" | "drive"; file_id: string; sheet?: string };

type StartedJob = {
	id: string;
	title: string;
	code: string;
	script_hash: string;
	source: Source | null;
	records: number | null;
	max_calls: number;
	calls: number;
	expires_at: number;
};

type JobAnswer =
	| { status: "ok"; job: StartedJob }
	| { status: "approved"; job_id: string; detail: string }
	| { status: "approval_required"; approval_id: string; job_id: string; max_calls: number; action: string; details: string; expires_at: number }
	| { status: "refused" | "failed"; detail: string };

/** The job a script runs now, per Pi conversation (tools run one at a time per conversation). */
type Running = {
	id: string;
	scriptHash: string;
	records: Record<string, string>[] | null;
	done: number;
	total: number | null;
	progress: (done: number, total: number | null) => Promise<void>;
};

const running = new Map<ConversationId, Running>();

/** What a write sends while a job's script runs: Orbit checks it against the job. */
export function jobRef(conversationId: ConversationId): { id: string; script_hash: string } | undefined {
	const job = running.get(conversationId);
	return job ? { id: job.id, script_hash: job.scriptHash } : undefined;
}

/** A job's script runs in this conversation now: its calls do not count against the run's limit. */
export function inJob(conversationId: ConversationId): boolean {
	return running.has(conversationId);
}

/** `2224` → `2,224`. */
const count = (value: number): string => value.toLocaleString("en-US");

/** The rows of a table as objects keyed by its first row (the header). */
export function recordsOf(rows: readonly string[][]): Record<string, string>[] {
	const [header, ...body] = rows;
	if (!header) return [];
	const keys = header.map((name, index) => name.trim() || `column ${index + 1}`);
	return body.filter((row) => row.some((cell) => cell.trim() !== "")).map((row) => Object.fromEntries(keys.map((key, index) => [key, row[index] ?? ""])));
}

/** The whole source file, as the driver reads it (the same permissions as read_chat_file and drive_read). */
async function loadRows(as: As, source: Source, thread: { conversation_id: string; root_message_id: string | null }): Promise<string[][]> {
	const rows: string[][] = [];
	let offset: number | null = 0;
	while (offset !== null) {
		let part: { rows?: string[][] | null; next_offset?: number | null };
		if (source.kind === "chat") {
			const query = new URLSearchParams({ offset: String(offset), extract: "true", limit: String(ROWS_PER_READ) });
			if (source.sheet) query.set("sheet", source.sheet);
			part = (await as("GET", `/chat/files/${encodeURIComponent(source.file_id)}/content?${query}`)) as FileContent;
		} else {
			const answer = (await as("POST", "/agent-tools/drive", {
				tool: "drive_read",
				arguments: { node_id: source.file_id, offset, limit: ROWS_PER_READ, ...(source.sheet ? { sheet: source.sheet } : {}) },
				...thread,
			})) as { status: string; detail?: string; result?: { rows?: string[][] | null; next_offset?: number | null } };
			if (answer.status !== "ok") throw new Error(`The source file cannot be read: ${answer.detail ?? answer.status}`);
			part = answer.result ?? {};
		}
		if (!part.rows) throw new Error("The source file has no rows: give a csv, tsv or spreadsheet as the job's source.");
		rows.push(...part.rows);
		if (rows.length > MAX_RECORDS + 1) throw new Error(`The source file has more than ${count(MAX_RECORDS)} rows: split the job.`);
		offset = part.next_offset ?? null;
	}
	return rows;
}

/** The job's message in the thread: posted once, then edited with the progress. */
function progressMessage(orbit: OrbitClient, state: ThreadState, job: StartedJob) {
	let messageId: string | undefined;
	let shown = 0;
	const body = (line: string) => `${line}\n_Reply **stop** in this thread to end the job._`;
	const post = async (text: string) => {
		messageId ??= await orbit.postMessage({
			workspaceId: state.workspaceId,
			conversationId: state.conversationId,
			rootMessageId: state.rootMessageId,
			body: body(text),
			nonce: `job:${job.id}`.slice(0, 64),
			agentUserId: state.agentUserId ?? null,
			silent: true,
		});
		return messageId;
	};
	const line = (done: number, total: number | null) => `⏳ **${job.title}**: ${count(done)}${total ? `/${count(total)}` : ""}`;
	return {
		start: () => post(`⏳ **${job.title}**: starting`),
		update: async (done: number, total: number | null) => {
			const now = Date.now();
			if (now - shown < PROGRESS_EVERY_MS) return;
			shown = now;
			const id = await post(line(done, total));
			await orbit.editMessage(id, state.workspaceId, body(line(done, total)), undefined, state.agentUserId ?? null);
		},
		end: async (text: string) => {
			const id = await post(text);
			await orbit.editMessage(id, state.workspaceId, text, undefined, state.agentUserId ?? null, true);
		},
	};
}

/** `records()` and `progress(done, total)` for a job's script. */
const JOB_GLOBALS: CodemodeGlobal[] = [
	{
		name: "records",
		execute: async (_args, api) => {
			const job = running.get(api.conversationId);
			if (!job?.records) throw new Error("This job has no source file: records() is empty.");
			return job.records;
		},
	},
	{
		name: "progress",
		spread: true,
		execute: async (args, api) => {
			const job = running.get(api.conversationId);
			if (!job) return undefined;
			const [done, total] = args as unknown[];
			if (typeof done !== "number" || !Number.isFinite(done)) throw new Error("progress(done, total) takes numbers.");
			job.done = done;
			if (typeof total === "number" && Number.isFinite(total)) job.total = total;
			await job.progress(job.done, job.total).catch((error: unknown) => console.warn("agent: the job's progress edit failed", String(error)));
			return undefined;
		},
	},
];

const jobCodemode = codemodeWith(JOB_GLOBALS);

const resultText = (result: ToolExecutionResult): string =>
	(result.output ?? [])
		.filter((part): part is { type: "text"; text: string } => part.type === "text")
		.map((part) => part.text)
		.join("\n");

/** The report the model gets at the end: what was done, what failed and what is left. */
export function jobReport(title: string, ok: boolean, done: number, total: number | null, calls: string, output: string): { line: string; text: string } {
	const of = total ? `${count(done)}/${count(total)}` : count(done);
	if (ok) {
		return {
			line: `✅ **${title}**: done, ${of}.`,
			text: `The job "${title}" is done: ${of} (${calls}). Tell the driver briefly what was done. The script's output:\n${untrusted("job output", output)}`,
		};
	}
	const left = total !== null ? Math.max(0, total - done) : null;
	return {
		line: `⛔ **${title}**: stopped at ${of}${left !== null ? `, ${count(left)} left` : ""}.`,
		text: `The job "${title}" stopped at its first error, after ${of} (${calls})${left !== null ? `; ${count(left)} are left` : ""}. Tell the driver what was done, what failed and why, and what is left; do not run it again on your own (a new plan_job for the rest needs their yes). The error and output:\n${untrusted("job error", output.slice(0, 4_000))}`,
	};
}

const PLAN_DESCRIPTION =
	"Plan a job: many writes of one kind (an import, a bulk update, many Drive items) that the driver approves ONCE, instead of one approval per request. Use it whenever a write with approval would run more than about 3 times. Orbit shows the driver your plan; after their yes, call run_job with the job_id. The job may only call `tool` (an integration tool, or drive_trash / drive_share), for an HTTP API only with `method` and `path` (no query string), and for GraphQL only the root mutations in `mutations`; at most max_calls times, for 60 minutes. Anything else asks again. " +
	"The script is JavaScript for code mode: it calls tools as in codemode, plus records() (the source file's rows as objects keyed by its header row) and progress(done, total). Write it to work in batches, call progress after each batch, and throw on the first error, also on GraphQL userErrors, so the job stops there. In a job, tools.use_integration resolves to JSON text {status, body}: JSON.parse it. For GraphQL, put the data in variables, never into the query text. Give the plan in plain words: summary (what and why), records, mapping (each source column you send → its target field), left_out (the columns you do not send, with why), samples (2–3 records as they will be sent).";

/** `plan_job` and `run_job`; `asDriver` and the client come from the Orbit tools. */
export function jobTools(asDriver: Driver, orbit: () => OrbitClient) {
	const ask = (api: ToolExecutionApi, context: Context, state: ThreadState, answer: Extract<JobAnswer, { status: "approval_required" }>) =>
		askForApproval(
			api,
			context,
			orbit(),
			state,
			answer,
			"for this job",
			`call run_job with job_id ${answer.job_id}`,
			// The answer first: a long plan folds behind "Show more" in chat.
			`<@${state.driverId}> I need your approval for one job. Reply **yes** in this thread within 30 minutes to run all of it: at most ${count(answer.max_calls)} calls of exactly this, for 60 minutes; anything else asks again. Reply **stop** at any time to end it.\n\n${answer.action}`,
		);

	const planJob = defineTool({
		name: PLAN_JOB_TOOL,
		description: PLAN_DESCRIPTION,
		callers: ["model"],
		parameters: Type.Object({
			title: Type.String({ minLength: 1, maxLength: 150, description: "For example: Import 2,224 customers into Shopify" }),
			summary: Type.String({ minLength: 1, maxLength: 2000, description: "What the job does and how, in plain words" }),
			tool: Type.String({ description: "The one write tool: <integration>__<tool> (for example shopify__api_request), drive_trash or drive_share" }),
			method: Type.Optional(Type.String({ description: "HTTP API: the method of every write, for example POST" })),
			path: Type.Optional(Type.String({ description: "HTTP API: the exact path of every write, starting with /" })),
			mutations: Type.Optional(Type.Array(Type.String(), { maxItems: 20, description: "GraphQL: the root mutation fields the job may run, for example [\"customerCreate\"]" })),
			max_calls: Type.Integer({ minimum: 1, maximum: 1000, description: "The most writes, for example the number of batches" }),
			records: Type.Optional(Type.Integer({ minimum: 0, description: "How many records the job handles" })),
			mapping: Type.Optional(Type.Array(Type.Object({ source: Type.String(), target: Type.String() }), { maxItems: 100, description: "Each source column → its target field (with how values change, for example yes → SUBSCRIBED)" })),
			left_out: Type.Optional(Type.Array(Type.String(), { maxItems: 100, description: "Each source column that is not sent, with why" })),
			samples: Type.Optional(Type.Array(Type.String(), { maxItems: 3, description: "2–3 records as they will be sent" })),
			source: Type.Optional(
				Type.Object({
					kind: Type.Union([Type.Literal("chat"), Type.Literal("drive")]),
					file_id: Type.String({ description: "A chat file's file_id, or a Drive file's node_id" }),
					sheet: Type.Optional(Type.String()),
				}),
			),
			code: Type.String({ minLength: 1, maxLength: 20000, description: "The job's script (JavaScript for code mode)" }),
		}),
		execute: (args, api, context) =>
			asDriver(api, context, async (as, state) => {
				const answer = (await as("POST", "/agent-tools/jobs", {
					...Object.fromEntries(Object.entries(args).filter(([, value]) => value !== undefined)),
					conversation_id: state.conversationId,
					root_message_id: state.rootMessageId,
				})) as JobAnswer;
				switch (answer.status) {
					case "approval_required":
						return ask(api, context, state, answer);
					case "approved":
						return { output: [{ type: "text", text: `The driver approved this job already: call run_job with job_id ${answer.job_id} now.` }] };
					case "ok":
						return { output: [{ type: "text", text: `Call run_job with job_id ${answer.job.id}.` }] };
					default:
						return { output: [{ type: "text", text: `${answer.status === "refused" ? "Refused" : "Failed"}: ${answer.detail}` }], isError: true };
				}
			}),
	});

	const runJob = defineTool({
		name: RUN_JOB_TOOL,
		description:
			"Run a job that the driver approved (after their yes to plan_job), by its job_id: Orbit runs the approved script as it was planned, posts its progress in the thread, and stops it at the first error. Then answer with the report.",
		callers: ["model"],
		parameters: Type.Object({ job_id: Type.String({ minLength: 1 }) }),
		execute: (args, api, context) =>
			asDriver(api, context, async (as, state) => {
				const thread = { conversation_id: state.conversationId, root_message_id: state.rootMessageId };
				const answer = (await as("POST", `/agent-tools/jobs/${encodeURIComponent(args.job_id)}/start`, thread)) as JobAnswer;
				if (answer.status === "approval_required") return ask(api, context, state, answer);
				if (answer.status !== "ok") {
					const detail = "detail" in answer ? answer.detail : "Orbit did not start the job.";
					return { output: [{ type: "text", text: `Refused: ${detail}` }], isError: true };
				}
				const job = answer.job;
				const message = progressMessage(orbit(), state, job);
				// Orbit answers with the calls the job made.
				const finish = (status: "done" | "failed", done: number, total: number | null, error?: string): Promise<{ calls?: number } | undefined> =>
					as("POST", `/agent-tools/jobs/${encodeURIComponent(job.id)}/finish`, { status, done, total, ...(error ? { error: error.slice(0, 1_000) } : {}) }).catch((failure: unknown) => {
						console.warn("agent: ending the job failed", String(failure));
						return undefined;
					});
				let records: Record<string, string>[] | null = null;
				try {
					records = job.source ? recordsOf(await loadRows(as, job.source, thread)) : null;
				} catch (error) {
					const detail = error instanceof Error ? error.message : String(error);
					await finish("failed", 0, job.records, detail);
					return { output: [{ type: "text", text: `The job did not start: ${detail}` }], isError: true };
				}
				const entry: Running = {
					id: job.id,
					scriptHash: job.script_hash,
					records,
					done: 0,
					total: job.records ?? records?.length ?? null,
					progress: (done, total) => message.update(done, total),
				};
				await message.start().catch((error: unknown) => console.warn("agent: posting the job's progress failed", String(error)));
				running.set(api.conversationId, entry);
				let result: ToolExecutionResult;
				try {
					result = await jobCodemode.execute({ code: `// @options: {"timeout_ms": ${JOB_TIMEOUT_MS}}\n${job.code}` }, api, context);
				} finally {
					running.delete(api.conversationId);
				}
				const ok = !result.isError;
				const output = resultText(result);
				const ended = await finish(ok ? "done" : "failed", entry.done, entry.total, ok ? undefined : output);
				const calls = `${count(ended?.calls ?? 0)} of at most ${count(job.max_calls)} approved calls`;
				const report = jobReport(job.title, ok, entry.done, entry.total, calls, output);
				await message.end(report.line).catch((error: unknown) => console.warn("agent: the job's last progress edit failed", String(error)));
				return { output: [{ type: "text", text: report.text }], isError: !ok };
			}),
	});

	return [planJob, runJob];
}
