/**
 * Routines, reminders and send later (`apps/server/src/scheduled/`). The tools only list and create, as the driver;
 * people change, pause and delete in Settings → Agentic → Routines and in the chat composer (`agent::scope` refuses
 * anything else from an agent session). Creating goes to `/agent-tools/scheduled`: a routine or a message posted as
 * the driver is made only after the driver's "yes" in the thread to exactly that action (Orbit keeps and checks the
 * approval); a reminder, a DM to the driver only, needs none. A routine runs later as its creator, with the creator's
 * permissions only, and without a new approval.
 */

import { Type } from "@earendil-works/pi-ai";
import { defineTool } from "@earendil-works/pi-durable";
import type { ToolExecutionApi, ToolExecutionResult } from "@earendil-works/pi-durable";
import type { Context } from "@earendil-works/chord/context";

import { untrusted } from "./files.ts";
import { askForApproval, type Driver } from "./integrations.ts";
import type { ThreadState } from "./docs.ts";
import type { OrbitClient } from "./orbit.ts";

type Json = Record<string, unknown>;
type As = (method: string, path: string, body?: unknown) => Promise<any>;

function ok(label: string, value: unknown): ToolExecutionResult {
	return { output: [{ type: "text", text: untrusted(label, value) }] };
}

function refused(text: string): ToolExecutionResult {
	return { output: [{ type: "text", text }], isError: true };
}

const where = Type.Optional(
	Type.String({
		description: 'Where to post: "here" (this conversation, the default), a conversation id (a channel in a message reads #channel(<id>)), or a channel name such as "#marketing".',
	}),
);
const at = Type.String({
	description: "When, as a local time in the driver's time zone (2026-11-11T17:00), or with an offset (2026-11-11T15:00:00Z).",
});
const timeZone = Type.Optional(Type.String({ description: "IANA time zone, e.g. Europe/Berlin. Default: the driver's profile time zone." }));

/** A conversation id for `post_in`: this one, an id, or a channel the driver is in, by name. */
export async function conversationFor(as: As, state: ThreadState, value: string | undefined): Promise<string | null> {
	const wanted = value?.trim() ?? "";
	if (!wanted || wanted.toLowerCase() === "here") return state.conversationId;
	const id = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i.exec(wanted);
	if (id) return id[0];
	const name = wanted.replace(/^#/, "").toLowerCase();
	const conversations = (await as("GET", "/chat/conversations")) as Json[];
	const found = conversations.find((conversation) => conversation.kind !== "dm" && String(conversation.name).toLowerCase() === name);
	return found ? String(found.id) : null;
}

function routineView(routine: Json): Json {
	const { id, name, prompt, schedule, time_zone, status, paused_reason, next_run_at, last_run_at, last_run_status, last_error, conversation_name, conversation_kind, creator_name } = routine;
	const post_in = conversation_kind === "dm" ? "a DM" : conversation_name ? `#${conversation_name}` : "nowhere (deleted)";
	return { id, name, instruction: prompt, schedule, time_zone, status, paused_reason, next_run_at: iso(next_run_at), last_run_at: iso(last_run_at), last_run_status, last_error, post_in, creator: creator_name };
}

function iso(value: unknown): string | null {
	return typeof value === "number" ? new Date(value).toISOString() : null;
}

/** What `/agent-tools/scheduled` answers. */
type Answer =
	| { status: "ok"; label: string; result: Json }
	| { status: "approval_required"; approval_id: string; action: string; expires_at: number }
	| { status: "refused" | "failed"; detail: string };

/** The tools; `asDriver` and the client come from the Orbit tools. */
export function scheduledTools(asDriver: Driver, orbit: () => OrbitClient) {
	/** Creates through Orbit, which asks the driver first for `schedule_routine` and `send_later`. */
	const create =
		(tool: string, args: Json, view: (result: Json) => unknown, postIn?: string) =>
		(api: ToolExecutionApi, context: Context): Promise<ToolExecutionResult> =>
			asDriver(api, context, async (as, state) => {
				const conversation = postIn === undefined ? undefined : await conversationFor(as, state, postIn);
				if (conversation === null) return refused(`I found no channel named ${postIn} that ${state.driverName ?? "the driver"} is in.`);
				const answer = (await as("POST", "/agent-tools/scheduled", {
					tool,
					arguments: Object.fromEntries(Object.entries({ ...args, conversation_id: conversation }).filter(([, value]) => value !== undefined)),
					conversation_id: state.conversationId,
					root_message_id: state.rootMessageId,
				})) as Answer;
				switch (answer.status) {
					case "ok":
						return ok(answer.label, view(answer.result));
					case "approval_required":
						return askForApproval(api, context, orbit(), state, answer, "to schedule this", `call ${tool} again with exactly the same arguments`);
					default:
						return refused(`${answer.status === "refused" ? "Refused" : "Failed"}: ${answer.detail}`);
				}
			});

	const scheduleRoutine = defineTool({
		name: "schedule_routine",
		description:
			"Create a routine: a recurring job that you do as the driver on a schedule, posting the result in a channel or DM. Use it only when the driver asks for something recurring. It needs the driver's approval each time: then stop, and after their yes call it again with exactly the same arguments. Turn plain words into a 5-field cron expression (minute hour day month weekday): \"every weekday at 9\" is `0 9 * * 1-5`, \"every Monday at 8:30\" is `30 8 * * 1`, \"every 2 hours\" is `0 */2 * * *`. At most every 15 minutes. The instruction must be complete on its own: it runs later without this conversation.",
		parameters: Type.Object({
			name: Type.String({ minLength: 1, maxLength: 80, description: "A short name, e.g. Daily standup summary" }),
			instruction: Type.String({ minLength: 1, maxLength: 4000, description: "What to do each time, in the driver's words, complete on its own" }),
			cron: Type.String({ description: "5-field cron expression in the time zone" }),
			time_zone: timeZone,
			post_in: where,
		}),
		execute: (args, api, context) =>
			create(
				"schedule_routine",
				{ name: args.name, prompt: args.instruction, schedule: args.cron, time_zone: args.time_zone },
				(routine) => ({ ...routineView(routine), manage: "Settings → Agentic → Routines" }),
				args.post_in ?? "",
			)(api, context),
	});

	const listRoutines = defineTool({
		name: "list_routines",
		description: "List the driver's routines (for an owner or admin: every routine of the workspace), with their schedule, status, last and next run.",
		parameters: Type.Object({}),
		execute: (_args, api, context) =>
			asDriver(api, context, async (as) => {
				const { routines } = (await as("GET", "/routines")) as { routines: Json[] };
				return ok("routines", routines.map(routineView));
			}),
	});

	const remindMe = defineTool({
		name: "remind_me",
		description: "Remind the driver once at a time: you send them a direct message then (with a notification). Only for the driver; not for other people.",
		parameters: Type.Object({ text: Type.String({ minLength: 1, maxLength: 4000, description: "What to remind them of, e.g. Call Ana" }), at, time_zone: timeZone }),
		execute: (args, api, context) =>
			create("remind_me", { body: args.text, send_at: args.at, time_zone: args.time_zone }, (reminder) => ({ id: reminder.id, text: reminder.body, at: iso(reminder.send_at) }))(api, context),
	});

	const sendLater = defineTool({
		name: "send_later",
		description:
			"Schedule a chat message from the driver, posted as them at a time. Use it only when the driver asked to send this exact message later; write the text they asked for. It needs the driver's approval each time: then stop, and after their yes call it again with exactly the same arguments. They can edit or cancel it in the composer's scheduled list.",
		parameters: Type.Object({ text: Type.String({ minLength: 1, maxLength: 4000 }), at, time_zone: timeZone, post_in: where }),
		execute: (args, api, context) =>
			create(
				"send_later",
				{ body: args.text, send_at: args.at, time_zone: args.time_zone },
				(message) => ({ id: message.id, at: iso(message.send_at), post_in: message.conversation_kind === "dm" ? "a DM" : `#${message.conversation_name}` }),
				args.post_in ?? "",
			)(api, context),
	});

	return [scheduleRoutine, listRoutines, remindMe, sendLater];
}
