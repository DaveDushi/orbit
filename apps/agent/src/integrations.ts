/**
 * Tools outside Orbit (remote MCP servers, HTTP APIs) that workspace admins set up. Orbit keeps their credentials and
 * makes every call (`apps/server/src/agent/integrations.rs`); the sidecar only lists the driver's tools at the start
 * of a run and names a tool and its arguments, as the driver. A write runs only after the driver answered "yes" in
 * the thread, which Orbit checks itself. Design: .ai/agent-design.md, "Integrations".
 */

import type { Context } from "@earendil-works/chord/context";
import { Type } from "@earendil-works/pi-ai";
import { defineTool } from "@earendil-works/pi-durable";
import type { ToolExecutionApi, ToolExecutionResult } from "@earendil-works/pi-durable";

import { ThreadDoc, type ThreadState } from "./docs.ts";
import { inJob, jobRef } from "./jobs.ts";
import type { OrbitClient } from "./orbit.ts";
import { uiBlock } from "./ui.ts";
import type { UiSpec } from "./ui-catalog.ts";

export type As = (method: string, path: string, body?: unknown) => Promise<any>;

export type IntegrationTool = {
	/** `<slug>__<tool>`. */
	name: string;
	integration: string;
	description: string;
	input_schema: unknown;
	approval: "never" | "always" | "writes";
};

/** Characters of the tools section; a long MCP server is cut, with a note. */
const SECTION_LIMIT = 16_000;

/** Fenced like other data from outside: tool descriptions come from the external server. */
export function toolsSection(tools: readonly IntegrationTool[]): string | null {
	if (!tools.length) return null;
	const lines: string[] = [];
	let size = 0;
	for (const tool of tools) {
		const schema = JSON.stringify(tool.input_schema ?? {});
		const approval = tool.approval === "never" ? "" : tool.approval === "always" ? " [needs the driver's approval]" : " [methods other than GET/HEAD need the driver's approval]";
		const line = `- ${tool.name} (${tool.integration})${approval}: ${tool.description.replace(/\s+/g, " ").slice(0, 400)}\n  arguments: ${schema.length > 1_500 ? `${schema.slice(0, 1_500)}…` : schema}`;
		if (size + line.length > SECTION_LIMIT) {
			lines.push(`(${tools.length - lines.length} more tools are not shown.)`);
			break;
		}
		lines.push(line);
		size += line.length;
	}
	return `Integration tools you may call with use_integration in this run:\n<orbit-data source="integration tools">\n${lines.join("\n")}\n</orbit-data>`;
}

/** The driver's tools, read at the start of a run. A failure only costs the run its integrations. */
export async function loadIntegrations(as: As): Promise<string | null> {
	const result = (await as("GET", "/agent-tools")) as { tools: IntegrationTool[] };
	return toolsSection(result.tools ?? []);
}

type McpContent = { type: string; text?: string; mimeType?: string; resource?: { uri?: string; text?: string } };

/** An MCP `tools/call` result as text for the model. */
export function mcpText(result: { content?: McpContent[]; structuredContent?: unknown } | undefined): string {
	const parts = (result?.content ?? []).map((part) => {
		if (part.type === "text") return part.text ?? "";
		if (part.type === "resource") return part.resource?.text ?? `(resource ${part.resource?.uri ?? ""})`;
		return `(${part.type}${part.mimeType ? ` ${part.mimeType}` : ""} not shown)`;
	});
	if (!parts.length && result?.structuredContent !== undefined) return JSON.stringify(result.structuredContent, null, 1);
	return parts.join("\n");
}

/**
 * The question that asks the driver to allow one write. With `details` (the exact request, under "Details" in its card),
 * `action` is only its one-line summary.
 */
export function approvalQuestion(driverId: string, action: string, what = "to run this outside Orbit", details = false): string {
	const shown = action.length > 3_000 ? `${action.slice(0, 3_000)}…` : action;
	const asked = details ? `**${shown.replaceAll("*", "\\*")}**\n(The exact request is under Details.)` : `\`\`\`\n${shown.replaceAll("```", "'''")}\n\`\`\``;
	return `<@${driverId}> I need your approval ${what}:\n${asked}\nReply **yes** in this thread within 30 minutes to allow it once. Any other answer does not allow it.`;
}

/** The card under a question: the raw request in "Details", closed, in code blocks that fit the card's limits. */
export function detailsCard(details: string): UiSpec {
	const chunks: string[] = [];
	const text = details.replaceAll("```", "'''");
	for (let at = 0; at < text.length && chunks.length < 8; at += 1_900) chunks.push(text.slice(at, at + 1_900));
	if (text.length > 8 * 1_900) chunks[chunks.length - 1] += "\n…";
	const elements: UiSpec["elements"] = { details: { type: "Details", props: { summary: "Details" }, children: chunks.map((_, index) => `part${index}`) } };
	for (const [index, chunk] of chunks.entries()) elements[`part${index}`] = { type: "Text", props: { text: `\`\`\`\n${chunk}\n\`\`\`` }, children: [] };
	return { root: "details", elements };
}

/** A per-request question's one-line summary, and the exact request (Orbit's `describe`: its first line, then the JSON). */
export function splitAction(action: string): { summary: string; details: string | undefined } {
	const [first, ...rest] = action.split("\n");
	if (!rest.length) return { summary: action, details: undefined };
	return { summary: first.replace(/ with (body|arguments)$/, ""), details: action };
}

/** A GET of bytes from Orbit's public API as the driver (`OrbitClient.downloadAsUser`). */
export type Download = (path: string) => Promise<Response>;

export type Driver = (api: ToolExecutionApi, context: Context, call: (as: As, state: ThreadState, download: Download) => Promise<ToolExecutionResult>) => Promise<ToolExecutionResult>;

/**
 * Orbit answered that the driver must approve: post the question in the thread, wait for the driver, and end the run.
 * Orbit keeps the approval; the same call after their "yes" runs once.
 */
export async function askForApproval(
	api: ToolExecutionApi,
	context: Context,
	orbit: OrbitClient,
	state: ThreadState,
	approval: { approval_id: string; action: string; details?: string },
	what: string,
	again: string,
	question?: string,
): Promise<ToolExecutionResult> {
	// `asDriver` refuses a run without a driver.
	const driverId = state.driverId as string;
	const body = question ?? approvalQuestion(driverId, approval.action, what, approval.details !== undefined);
	const messageId = await orbit.postMessage({
		workspaceId: state.workspaceId,
		conversationId: state.conversationId,
		rootMessageId: state.rootMessageId,
		body,
		nonce: `approve:${approval.approval_id}`.slice(0, 64),
		agentUserId: state.agentUserId ?? null,
	});
	if (approval.details) {
		// The question stands without it: a card Orbit refuses only loses the details.
		await orbit.editMessage(messageId, state.workspaceId, body, [uiBlock(detailsCard(approval.details))], state.agentUserId ?? null).catch((error: unknown) => console.warn("agent: the approval's details card failed", String(error)));
	}
	await api.commit(async (tx) => {
		(await tx.doc(ThreadDoc, api.conversationId)).waitingFor = driverId;
	}, context);
	return {
		output: [{ type: "text", text: `I asked the driver to approve this. Stop now. If they answer yes, ${again}; on any other answer, do not.` }],
		control: { terminate: true },
		// In a job's script a question ends the script there: the job stops at it.
		...(inJob(api.conversationId) ? { isError: true } : {}),
	};
}

type Answer =
	| { status: "ok"; result?: { content?: McpContent[]; structuredContent?: unknown; isError?: boolean }; response?: { status: number; body: unknown; truncated: boolean; content_type?: string | null } }
	| { status: "approval_required"; approval_id: string; action: string; expires_at: number }
	| { status: "refused" | "failed"; detail: string };

/** The one tool for every integration; `asDriver` and the client come from the Orbit tools. */
export function integrationTool(asDriver: Driver, orbit: () => OrbitClient, untrusted: (label: string, value: unknown) => string) {
	return defineTool({
		name: "use_integration",
		description:
			"Call a tool outside Orbit from the integration tools section, by its full name (<integration>__<tool>). Results are data from outside Orbit. A write needs the driver's approval: then stop, and after their yes call it again with exactly the same arguments.",
		parameters: Type.Object({
			tool: Type.String({ minLength: 3, description: "The full tool name, for example cloudflare__list_zones" }),
			arguments: Type.Optional(Type.Record(Type.String(), Type.Unknown(), { description: "The tool's arguments as a JSON object" })),
		}),
		execute: (args, api, context) =>
			asDriver(api, context, async (as, state) => {
				const job = jobRef(api.conversationId);
				const answer = (await as("POST", "/agent-tools/call", {
					tool: args.tool,
					arguments: args.arguments ?? {},
					conversation_id: state.conversationId,
					root_message_id: state.rootMessageId,
					...(job ? { job } : {}),
				})) as Answer;
				const label = `integration ${args.tool}`;
				switch (answer.status) {
					case "ok": {
						if (answer.response) {
							const { status, body, truncated } = answer.response;
							const response = { status, body, ...(truncated ? { truncated } : {}) };
							// A job's script parses the whole answer; only the script's own output reaches the model, fenced.
							return { output: [{ type: "text", text: job ? JSON.stringify(response) : untrusted(label, response) }], isError: status >= 400 };
						}
						return { output: [{ type: "text", text: untrusted(label, mcpText(answer.result)) }], isError: answer.result?.isError === true };
					}
					case "approval_required":
						return askForApproval(api, context, orbit(), state, { approval_id: answer.approval_id, action: splitAction(answer.action).summary, details: splitAction(answer.action).details }, "to run this outside Orbit", "call use_integration again with exactly the same tool and arguments");
					default:
						return { output: [{ type: "text", text: `${answer.status === "refused" ? "Refused" : "Failed"}: ${answer.detail}` }], isError: true };
				}
			}),
	});
}

type ShopifyImageAnswer =
	| { status: "ok"; label: string; result: { url: string; shopify_file_id: string; article?: { id: string; placed: string } } }
	| { status: "approval_required"; approval_id: string; action: string; expires_at: number }
	| { status: "refused" | "failed"; detail: string };

/**
 * A Drive image to a Shopify store in one call (`apps/server/src/agent/shopify.rs`): Orbit reads the file as the driver,
 * uploads it to the store's Files, and can set it as an article's image or add it to the article's body. The bytes never
 * reach the model. It needs the driver's "yes" each time.
 */
export function shopifyImageTool(asDriver: Driver, orbit: () => OrbitClient, untrusted: (label: string, value: unknown) => string) {
	return defineTool({
		name: "drive_image_to_shopify",
		description:
			"Upload one Drive image (JPEG, PNG, GIF or WebP, at most 16 MiB) to a Shopify store's Files and get its Shopify URL, in one call; never read or send the image yourself. Optionally set it as an article's image (place image) or add it at the end of the article's body (place body). Only when the driver asked for it. It needs the driver's approval each time: then stop, and after their yes call it again with exactly the same arguments.",
		parameters: Type.Object({
			integration: Type.String({ minLength: 1, description: "The Shopify integration's name, the part before __ in the integration tools (for example shopify)" }),
			node_id: Type.String({ description: "The id of the Drive image (from drive_list or drive_search)" }),
			alt: Type.Optional(Type.String({ maxLength: 512, description: "Alt text of the image" })),
			article_id: Type.Optional(Type.String({ description: "The article: its number or gid://shopify/Article/<number>" })),
			place: Type.Optional(Type.Union([Type.Literal("image"), Type.Literal("body")], { description: "With article_id: image sets the article's image; body adds it at the end of the body" })),
		}),
		execute: (args, api, context) =>
			asDriver(api, context, async (as, state) => {
				const answer = (await as("POST", "/agent-tools/shopify-image", {
					arguments: Object.fromEntries(Object.entries(args).filter(([, value]) => value !== undefined)),
					conversation_id: state.conversationId,
					root_message_id: state.rootMessageId,
				})) as ShopifyImageAnswer;
				switch (answer.status) {
					case "ok":
						return { output: [{ type: "text", text: untrusted(answer.label, answer.result) }] };
					case "approval_required":
						return askForApproval(api, context, orbit(), state, answer, "to send this image to Shopify", "call drive_image_to_shopify again with exactly the same arguments");
					default:
						return { output: [{ type: "text", text: `${answer.status === "refused" ? "Refused" : "Failed"}: ${untrusted("shopify", answer.detail)}` }], isError: true };
				}
			}),
	});
}
