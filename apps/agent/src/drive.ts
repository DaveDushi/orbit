/**
 * Orbit Drive tools. Every call goes to `/agent-tools/drive` as the driver (`apps/server/src/agent/drive.rs`), which runs
 * the Drive tools of MCP clients with the driver's permissions. Moving to the trash and making a public link run only
 * after the driver's "yes" in the thread: Orbit keeps that approval and checks it, as for integration writes.
 */

import type { Context } from "@earendil-works/chord/context";
import { Type } from "@earendil-works/pi-ai";
import { defineTool } from "@earendil-works/pi-durable";
import type { ToolExecutionApi, ToolExecutionResult } from "@earendil-works/pi-durable";

import { fileReadText, READ_CHARS, untrusted, type FileContent } from "./files.ts";
import { askForApproval, type Driver } from "./integrations.ts";
import type { OrbitClient } from "./orbit.ts";

type Answer =
	| { status: "ok"; label: string; result: Record<string, unknown> }
	| { status: "approval_required"; approval_id: string; action: string; expires_at: number }
	| { status: "refused" | "failed"; detail: string };

/** A drive_read result as the chat and page file readers show it. */
export function driveReadText(result: Record<string, any>, offset: number): string {
	const content: FileContent = {
		file: { id: String(result.file?.id), file_name: String(result.file?.name), kind: "file", size_bytes: Number(result.file?.size ?? 0) },
		sheets: result.sheets ?? [],
		sheet: result.sheet ?? null,
		rows: result.rows ?? null,
		row_numbers: null,
		text: result.text ?? null,
		next_offset: result.next_offset ?? null,
		no_text: result.no_text === true,
	};
	return fileReadText(content, offset);
}

const nodeId = Type.String({ description: "The id of a Drive file or folder (from drive_list or drive_search)" });
const offset = Type.Optional(Type.Number({ minimum: 0 }));

/** The Drive tools; `asDriver` and the client come from the Orbit tools. */
export function driveTools(asDriver: Driver, orbit: () => OrbitClient) {
	/** `tool` is the server's name; `again` names the agent's tool for a second call after a "yes". */
	const run =
		(tool: string, again: string, args: Record<string, unknown>, format?: (result: Record<string, any>) => string) =>
		(api: ToolExecutionApi, context: Context): Promise<ToolExecutionResult> =>
			asDriver(api, context, async (as, state) => {
				const answer = (await as("POST", "/agent-tools/drive", {
					tool,
					arguments: Object.fromEntries(Object.entries(args).filter(([, value]) => value !== undefined)),
					conversation_id: state.conversationId,
					root_message_id: state.rootMessageId,
				})) as Answer;
				switch (answer.status) {
					case "ok":
						return { output: [{ type: "text", text: untrusted(answer.label, format ? format(answer.result) : answer.result) }] };
					case "approval_required":
						return askForApproval(api, context, orbit(), state, answer, "for this in Orbit Drive", `call ${again} again with exactly the same arguments`);
					default:
						return { output: [{ type: "text", text: `${answer.status === "refused" ? "Refused" : "Failed"}: ${answer.detail}` }], isError: true };
				}
			});

	return [
		defineTool({
			name: "drive_list",
			description: "List the files and folders in a folder of the workspace's Drive (folders first); without folder_id, the top level. A result with a next_offset has more.",
			parameters: Type.Object({ folder_id: Type.Optional(Type.String()), offset }),
			execute: (args, api, context) => run("drive_list", "drive_list", args)(api, context),
		}),
		defineTool({
			name: "drive_search",
			description: "Find Drive files and folders by part of their name (any case), with the path of their folder.",
			parameters: Type.Object({
				query: Type.Optional(Type.String()),
				kind: Type.Optional(Type.Union([Type.Literal("file"), Type.Literal("folder")])),
				offset,
			}),
			execute: (args, api, context) => run("drive_search", "drive_search", args)(api, context),
		}),
		defineTool({
			name: "drive_info",
			description: "Describe one Drive file or folder and the folders above it.",
			parameters: Type.Object({ node_id: nodeId }),
			execute: (args, api, context) => run("drive_info", "drive_info", args)(api, context),
		}),
		defineTool({
			name: "drive_read",
			description:
				"Read one part of a Drive file: text, csv, tsv, spreadsheets, Word and PDF (at most 16 MiB). Images and scans have no text you can read. A result with a next_offset has more: call again with that offset. Offsets count rows for spreadsheets, csv and tsv, and characters otherwise. For a workbook, name the sheet.",
			parameters: Type.Object({ node_id: nodeId, sheet: Type.Optional(Type.String()), offset }),
			execute: (args, api, context) =>
				run("drive_read", "drive_read", { ...args, limit: READ_CHARS }, (result) => driveReadText(result, args.offset ?? 0))(api, context),
		}),
		defineTool({
			name: "drive_create_folder",
			description: "Create a folder in the Drive (without parent_id: at the top level). Only when the driver asked for it.",
			parameters: Type.Object({ name: Type.String({ minLength: 1 }), parent_id: Type.Optional(Type.String()) }),
			execute: (args, api, context) => run("drive_create_folder", "drive_create_folder", args)(api, context),
		}),
		defineTool({
			name: "drive_save_text",
			description:
				"Save a new text file (at most 48 KiB) into a Drive folder. Only when the driver asked for it. If the name is used, nothing is saved, unless on_conflict is `rename` (name (2).txt). It never replaces a file.",
			parameters: Type.Object({
				name: Type.String({ minLength: 1, description: "The file name, with its extension" }),
				text: Type.String(),
				parent_id: Type.Optional(Type.String()),
				type: Type.Optional(Type.Union([Type.Literal("plain"), Type.Literal("markdown"), Type.Literal("csv"), Type.Literal("json")])),
				on_conflict: Type.Optional(Type.Union([Type.Literal("fail"), Type.Literal("rename")])),
			}),
			execute: (args, api, context) => run("drive_upload_text", "drive_save_text", args)(api, context),
		}),
		defineTool({
			name: "drive_move",
			description: "Move a Drive file or folder into another folder (without parent_id: to the top level). Only when the driver asked for it.",
			parameters: Type.Object({ node_id: nodeId, parent_id: Type.Optional(Type.String()) }),
			execute: (args, api, context) => run("drive_move", "drive_move", args)(api, context),
		}),
		defineTool({
			name: "drive_rename",
			description: "Rename a Drive file or folder. Only when the driver asked for it.",
			parameters: Type.Object({ node_id: nodeId, name: Type.String({ minLength: 1 }) }),
			execute: (args, api, context) => run("drive_rename", "drive_rename", args)(api, context),
		}),
		defineTool({
			name: "drive_trash",
			description:
				"Move a Drive file or folder (with everything in it) to the trash, where members can restore it for 30 days. Only when the driver asked for it. It needs the driver's approval each time: then stop, and after their yes call it again with exactly the same arguments.",
			parameters: Type.Object({ node_id: nodeId }),
			execute: (args, api, context) => run("drive_trash", "drive_trash", args)(api, context),
		}),
		defineTool({
			name: "drive_share",
			description:
				"Make a public link to a Drive file or folder: anyone with the link can open it without signing in. Only when the driver asked for it. It needs the driver's approval each time: then stop, and after their yes call it again with exactly the same arguments.",
			parameters: Type.Object({
				node_id: nodeId,
				expires_in_days: Type.Optional(Type.Integer({ minimum: 1, maximum: 365, description: "Without it, the link works until someone turns it off" })),
			}),
			execute: (args, api, context) => run("drive_share", "drive_share", args)(api, context),
		}),
	];
}
