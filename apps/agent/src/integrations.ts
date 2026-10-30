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
import type { OrbitClient } from "./orbit.ts";

type As = (method: string, path: string, body?: unknown) => Promise<any>;

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

/** The question that asks the driver to allow one write. */
export function approvalQuestion(driverId: string, action: string): string {
	const shown = action.length > 3_000 ? `${action.slice(0, 3_000)}…` : action;
	return `<@${driverId}> I need your approval to run this outside Orbit:\n\`\`\`\n${shown.replaceAll("```", "'''")}\n\`\`\`\nReply **yes** in this thread within 30 minutes to allow it once. Any other answer does not allow it.`;
}

type Driver = (api: ToolExecutionApi, context: Context, call: (as: As, state: ThreadState) => Promise<ToolExecutionResult>) => Promise<ToolExecutionResult>;

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
				// `asDriver` refuses a run without a driver.
				const driverId = state.driverId as string;
				const answer = (await as("POST", "/agent-tools/call", {
					tool: args.tool,
					arguments: args.arguments ?? {},
					conversation_id: state.conversationId,
					root_message_id: state.rootMessageId,
				})) as Answer;
				const label = `integration ${args.tool}`;
				switch (answer.status) {
					case "ok": {
						if (answer.response) {
							const { status, body, truncated } = answer.response;
							return { output: [{ type: "text", text: untrusted(label, { status, body, ...(truncated ? { truncated } : {}) }) }], isError: status >= 400 };
						}
						return { output: [{ type: "text", text: untrusted(label, mcpText(answer.result)) }], isError: answer.result?.isError === true };
					}
					case "approval_required": {
						await orbit().postMessage({
							workspaceId: state.workspaceId,
							conversationId: state.conversationId,
							rootMessageId: state.rootMessageId,
							body: approvalQuestion(driverId, answer.action),
							nonce: `approve:${answer.approval_id}`.slice(0, 64),
						});
						await api.commit(async (tx) => {
							(await tx.doc(ThreadDoc, api.conversationId)).waitingFor = driverId;
						}, context);
						return {
							output: [{ type: "text", text: "I asked the driver to approve this. Stop now. If they answer yes, call use_integration again with exactly the same tool and arguments." }],
							control: { terminate: true },
						};
					}
					default:
						return { output: [{ type: "text", text: `${answer.status === "refused" ? "Refused" : "Failed"}: ${answer.detail}` }], isError: true };
				}
			}),
	});
}
