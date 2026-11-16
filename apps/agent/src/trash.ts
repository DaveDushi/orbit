/**
 * Docs pages and tasks to the Trash. The call goes to `/agent-tools/trash` as the driver (`apps/server/src/agent/trash.rs`),
 * which runs the web's own soft delete (a page with its sub-pages, a task with its sub-issues) only after the driver's
 * "yes" in the thread, for exactly these arguments. One call takes many pages and tasks, so a batch needs one approval.
 */

import { Type } from "@earendil-works/pi-ai";
import { defineTool } from "@earendil-works/pi-durable";

import { untrusted } from "./files.ts";
import { askForApproval, type Driver } from "./integrations.ts";
import type { OrbitClient } from "./orbit.ts";

type Answer =
	| { status: "ok"; label: string; result: Record<string, unknown> }
	| { status: "approval_required"; approval_id: string; action: string; expires_at: number }
	| { status: "refused" | "failed"; detail: string };

/** `move_to_trash`; `asDriver` and the client come from the Orbit tools. */
export function trashTool(asDriver: Driver, orbit: () => OrbitClient) {
	return defineTool({
		name: "move_to_trash",
		description:
			"Move Docs pages (with their sub-pages) and tasks (with their sub-issues) to the Trash, where members can restore them for 30 days. Give all of them in one call: the driver approves the whole list at once. Only when the driver asked for it. It needs the driver's approval each time: then stop, and after their yes call it again with exactly the same arguments. It never deletes forever.",
		parameters: Type.Object({
			page_ids: Type.Optional(Type.Array(Type.String(), { description: "Ids of Docs pages (from list_pages or search_pages)" })),
			task_ids: Type.Optional(Type.Array(Type.String(), { description: "Ids of tasks (from search_tasks or get_task)" })),
		}),
		execute: (args, api, context) =>
			asDriver(api, context, async (as, state) => {
				const answer = (await as("POST", "/agent-tools/trash", {
					tool: "move_to_trash",
					arguments: Object.fromEntries(Object.entries(args).filter(([, value]) => value !== undefined)),
					conversation_id: state.conversationId,
					root_message_id: state.rootMessageId,
				})) as Answer;
				switch (answer.status) {
					case "ok":
						return { output: [{ type: "text", text: untrusted(answer.label, answer.result) }] };
					case "approval_required":
						return askForApproval(api, context, orbit(), state, answer, "to move this to the Trash", "call move_to_trash again with exactly the same arguments");
					default:
						return { output: [{ type: "text", text: `${answer.status === "refused" ? "Refused" : "Failed"}: ${answer.detail}` }], isError: true };
				}
			}),
	});
}
