/**
 * `sandbox_shell`: a custom agent's own sandbox (docs/agent-sandbox-design.md; deploy/agent-sandbox/broker.py). One
 * gVisor sandbox per agent, chat thread and driver, with /work kept between calls. Orbit judges each call: it checks the
 * driver, the agent and its budget, and signs a ticket for the broker with the call token and the proxy password sealed
 * inside, so this process never sees them. Code in the sandbox reaches the internet only through the egress proxy, which
 * asks Orbit for every host.
 *
 * A host nobody allowed yet fails fast in the sandbox (403) and comes back with the call's end: this tool then posts one
 * card in the thread (Allow once / Allow always / Deny, answered on Orbit's own route) and ends the run waiting for the
 * driver, as approvals do. The answer comes back as an `action` input of the driver (`ui.ts`, `actionText`).
 *
 * A call that ends without the broker's answer (the run was stopped, the broker failed or took too long) is ended in Orbit
 * here as interrupted: its token and proxy password die at once. A call cut by a restart of this process is reported
 * to the model by Pi as interrupted (the tool is not `replay: "safe"`); the broker sees the closed connection and kills it.
 */

import { createHash } from "node:crypto";

import { Type } from "@earendil-works/pi-ai";
import { defineTool } from "@earendil-works/pi-durable";
import type { ToolExecutionResult } from "@earendil-works/pi-durable";

import { ThreadDoc } from "./docs.ts";
import { untrusted } from "./files.ts";
import type { Driver } from "./integrations.ts";
import { inJob } from "./jobs.ts";
import { ending, shown } from "./media.ts";
import { OrbitError, type OrbitClient } from "./orbit.ts";
import { brokerRequest, runResult, SandboxError } from "./sandbox.ts";

/** How long the broker may take beyond the call's own timeout (start, eviction wait, end report). */
const BROKER_GRACE_SECONDS = 60;
const OUTPUT_MAX = 64 * 1024;
const MAX_SEND_FILES = 4;
const MAX_SEND_BYTES = 20 * 1024 * 1024;

type EgressPending = { id: string; host: string; port: number };
type EgressDenied = { host: string; port: number; reason: string };

const fail = (text: string): ToolExecutionResult => ({ output: [{ type: "text", text }], isError: true });

/** The card's question: the hosts, and the model's reason in one short line (its own words, which hostile content may
 * have written: cut, flattened, and without mention or markdown characters, so it cannot ping people or fake a bold
 * answer or a link). */
export function egressQuestion(driverId: string, agentName: string, pending: readonly EgressPending[], purpose: string | undefined): string {
	const hosts = pending.map((request) => `**${request.host}:${request.port}**`).join(", ");
	const why = purpose?.replace(/\s+/g, " ").replace(/[<>[\]()*_`~|#\\]/g, "").trim().slice(0, 200);
	return `<@${driverId}> ${agentName} wants to connect to ${hosts}${why ? ` (for: ${why})` : ""}. The question waits 30 minutes; after that I ask again at the next try.`;
}

/** The tool; `socket` is the agent sandbox broker's socket (absent: no tool). */
export function sandboxTools(asDriver: Driver, orbit: () => OrbitClient, socket: () => string | undefined) {
	const sandboxShell = defineTool({
		name: "sandbox_shell",
		description:
			"Run a bash command in your own sandbox (Ubuntu with python3, node, git, curl, jq, ripgrep), in /work: this thread's folder (4 GiB), kept between calls. The sandbox stops after 30 minutes without calls; /work stays. A call's processes stop at its end, except ones started with setsid. Install tools into /work (pip install --user, npm --prefix /work/.local). Internet: HTTPS only, through a proxy that is already set. A host that nobody allowed yet fails with 403 egress_approval_required: this tool then asks the driver, and you stop and wait; name every host the command needs in hosts, so one question asks for all of them. Orbit API: $ORBIT_URL with the header Authorization: Bearer $ORBIT_TOKEN, as the driver and with your tools only. No secrets are in the sandbox. Answers: exit code, stdout, the end of stderr. If a call was interrupted, check /work before you run it again. To give the driver a result file (a chart, a CSV, a PDF), name it in send_files: after the command it is posted in this thread as your file (up to 4 files, 20 MB each); never paste file bytes or base64 into your answer.",
		parameters: Type.Object({
			command: Type.String({ minLength: 1, maxLength: 20_000, description: "bash; it runs in /work" }),
			timeout_seconds: Type.Optional(Type.Integer({ minimum: 1, maximum: 900, description: "Default 120" })),
			purpose: Type.Optional(Type.String({ maxLength: 200, description: "Why the command needs the internet, in a few words; the driver sees it when a host needs their yes" })),
			hosts: Type.Optional(Type.Array(Type.String({ minLength: 1, maxLength: 253 }), { maxItems: 10, description: "Every host name the command will reach (for example pypi.org, files.pythonhosted.org): when one needs the driver's yes, one card asks for all of them" })),
			send_files: Type.Optional(Type.Array(Type.String({ minLength: 1, maxLength: 500 }), { maxItems: MAX_SEND_FILES, description: "Files (in /work, or a path from /) to post in this thread after the command, when it ended with exit code 0" })),
		}),
		execute: (args, api, context) =>
			asDriver(api, context, async (_as, state) => {
				const path = socket();
				if (!path) return fail("The agent sandbox is not set up on this server.");
				if (!state.agentUserId || !state.threadKey || !state.driverId) return fail("Only custom agents have a sandbox.");
				const driverId = state.driverId;
				const timeout = args.timeout_seconds ?? 120;
				let ticket;
				try {
					ticket = await orbit().sandboxCall({
						workspace_id: state.workspaceId,
						user_id: driverId,
						agent_user_id: state.agentUserId,
						thread_key: state.threadKey,
						timeout_seconds: timeout,
						script_sha256: createHash("sha256").update(args.command).digest("hex"),
						...(args.hosts?.length ? { hosts: args.hosts } : {}),
					});
				} catch (error) {
					if (error instanceof OrbitError && error.status === 403) return fail(`Orbit refused the sandbox: ${error.problem?.detail ?? error.problem?.code ?? "no reason"}`);
					throw error;
				}

				// The call stops when the run stops, or when the broker takes far longer than the call's own timeout.
				const stop = new AbortController();
				const stopped = () => stop.abort(new SandboxError("the run was stopped"));
				context.abortSignal?.addEventListener("abort", stopped, { once: true });
				const timer = setTimeout(() => stop.abort(new SandboxError(`the sandbox did not answer within ${timeout + BROKER_GRACE_SECONDS} s`)), (timeout + BROKER_GRACE_SECONDS) * 1_000);
				let result;
				try {
					result = runResult(await brokerRequest(path, { op: "run", ticket: ticket.ticket, script: args.command, stdout_max: OUTPUT_MAX, stderr_max: OUTPUT_MAX }, undefined, stop.signal));
				} catch (error) {
					if (!(error instanceof SandboxError)) throw error;
					const reason = error.message.replace(/\.$/, "").slice(0, 200);
					// The broker reports the ends it saw itself; Orbit counts only the first end.
					await orbit()
						.endSandboxCall(ticket.call_id, reason)
						.catch((failure: unknown) => console.warn("agent: ending an interrupted sandbox call failed", String(failure)));
					if (context.abortSignal?.aborted) throw error;
					return fail(`The sandbox call was interrupted: ${reason}. It may have partly run: check /work before you run it again.`);
				} finally {
					clearTimeout(timer);
					context.abortSignal?.removeEventListener("abort", stopped);
				}

				const end = result.end;
				const interrupted = typeof end.interrupted === "string" ? end.interrupted : null;
				// In the sandbox a refused host is only a 403 (curl: exit 56); Orbit's reason comes with the call's end.
				const denied = (Array.isArray(end.egress_denied) ? end.egress_denied : []) as EgressDenied[];
				const text = untrusted("sandbox shell", {
					result: interrupted ? `interrupted: ${interrupted} (it may have partly run: check /work before you run it again)` : ending(result, timeout),
					...(typeof end.restarted === "string" ? { sandbox_restarted: end.restarted } : {}),
					...(denied.length ? { internet_refused: denied.map((refused) => `${refused.host}:${refused.port} (${refused.reason})`).join("; ") } : {}),
					stdout: shown(result.stdout, 8_000, "start", result.stdoutTruncated),
					stderr: shown(result.stderr, 3_000, "end", result.stderrTruncated),
				});
				const pending = (Array.isArray(end.egress_pending) ? end.egress_pending : []) as EgressPending[];
				if (!pending.length) {
					if (!args.send_files?.length || interrupted || result.exit !== 0) return { output: [{ type: "text", text }] };
					const sent = await sendFiles(state as SandboxState, path, args.send_files, context.abortSignal);
					return { output: [{ type: "text", text }, { type: "text", text: sent.text }], ...(sent.failed ? { isError: true } : {}) };
				}

				// One card per call (the nonce), then the run waits for the driver, as for an approval.
				await orbit().postMessage({
					workspaceId: state.workspaceId,
					conversationId: state.conversationId,
					rootMessageId: state.rootMessageId,
					body: egressQuestion(driverId, state.agentName ?? "The agent", pending, args.purpose),
					nonce: `egress:${ticket.call_id}`,
					agentUserId: state.agentUserId,
				});
				await api.commit(async (tx) => {
					(await tx.doc(ThreadDoc, api.conversationId)).waitingFor = driverId;
				}, context);
				const hosts = pending.map((request) => `${request.host}:${request.port}`).join(", ");
				return {
					output: [
						{ type: "text", text },
						{ type: "text", text: `The command needs internet access to ${hosts}, which the driver must allow first. I asked them in the thread. Stop now. When they allow it, run the command again; when they deny it, do not try that host again.` },
					],
					control: { terminate: true },
					// In a job's script the question ends the script there, as approvals do.
					...(inJob(api.conversationId) ? { isError: true } : {}),
				};
			}),
	});
	type SandboxState = { workspaceId: string; conversationId: string; rootMessageId: string | null; agentUserId: string; threadKey: string; driverId: string };

	/** Reads one file of the thread's sandbox: a call of its own (a new ticket, so Orbit checks the driver again). */
	async function readFile(state: SandboxState, socketPath: string, file: string, signal: AbortSignal | undefined): Promise<Buffer> {
		const script = 'if [ ! -f "$1" ]; then echo "not a file" >&2; exit 3; fi; if [ "$(stat -L -c %s -- "$1")" -gt "$2" ]; then echo "larger than 20 MB" >&2; exit 4; fi; cat -- "$1"';
		const ticket = await orbit().sandboxCall({
			workspace_id: state.workspaceId,
			user_id: state.driverId,
			agent_user_id: state.agentUserId,
			thread_key: state.threadKey,
			timeout_seconds: 60,
			script_sha256: createHash("sha256").update(script).digest("hex"),
		});
		const stop = AbortSignal.any([AbortSignal.timeout((60 + BROKER_GRACE_SECONDS) * 1_000), ...(signal ? [signal] : [])]);
		try {
			const read = runResult(await brokerRequest(socketPath, { op: "run", ticket: ticket.ticket, script, args: [file, String(MAX_SEND_BYTES)], stdout_max: MAX_SEND_BYTES, stderr_max: 1024 }, undefined, stop));
			if (read.exit !== 0 || read.stdoutTruncated) throw new SandboxError(read.stderr.toString("utf8").trim().slice(0, 200) || ending(read, 60));
			return read.stdout;
		} catch (error) {
			if (error instanceof SandboxError) await orbit().endSandboxCall(ticket.call_id, "the file read stopped").catch(() => undefined);
			throw error;
		}
	}

	/** Posts `files` of the sandbox in the run's thread as one message of the agent, with the files attached. */
	async function sendFiles(state: SandboxState, socketPath: string, files: readonly string[], signal: AbortSignal | undefined): Promise<{ text: string; failed: boolean }> {
		const ids: string[] = [];
		const names: string[] = [];
		for (const file of files.slice(0, MAX_SEND_FILES)) {
			const path = file.startsWith("/") ? file : `/work/${file}`;
			const name = path.split("/").pop() || "file";
			try {
				const bytes = await readFile(state, socketPath, path, signal);
				ids.push(await orbit().uploadAgentFile(state.workspaceId, state.agentUserId, name, bytes));
				names.push(name);
			} catch (error) {
				if (!(error instanceof SandboxError || error instanceof OrbitError)) throw error;
				const reason = error instanceof OrbitError ? (error.problem?.detail ?? `Orbit answered ${error.status}`) : error.message;
				return { text: `Not sent: ${file}: ${reason}. ${ids.length ? "Nothing was posted." : ""}`.trim(), failed: true };
			}
		}
		await orbit().postMessage({
			workspaceId: state.workspaceId,
			conversationId: state.conversationId,
			rootMessageId: state.rootMessageId,
			body: "",
			nonce: `files:${ids.join(",")}`,
			agentUserId: state.agentUserId,
			fileIds: ids,
		});
		return { text: `Posted in the thread: ${names.join(", ")}. The driver sees the file${names.length > 1 ? "s" : ""}; do not paste the content again.`, failed: false };
	}

	return [sandboxShell];
}
