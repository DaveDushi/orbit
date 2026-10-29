/**
 * AGENT_MOCK=1: a scripted model for tests and demos without a ChatGPT account. It reads the newest instruction,
 * calls one real Orbit tool for it, and then reports the tool's result. Everything else (Pi Durable, the tools,
 * Orbit's permissions, chat) is the real code path.
 *
 * Understood instructions (quotes required around titles and text):
 *   create task "Title"            rename task <id|it> to "Title"       move task <id|it> to <Status>
 *   create page "Title" with "Text"   append "Text" to page <id|it>       read page <id|it>
 *   search tasks "words"            list projects                         read messages
 *   ask @Name "Question"           show prompt (answers with the system prompt, to check names and team instructions)
 */

import { createModels, type AssistantMessage, type Models } from "@earendil-works/pi-ai";
import { fauxAssistantMessage, fauxProvider, fauxText, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";

import { PROVIDER } from "./models.ts";

type Message = { role: string; content?: unknown; toolName?: string; isError?: boolean; sections?: Record<string, string | null> };

function textOf(content: unknown): string {
	if (typeof content === "string") return content;
	if (!Array.isArray(content)) return "";
	return content.map((part: { type?: string; text?: string }) => (part.type === "text" ? (part.text ?? "") : "")).join("");
}

/** The current system prompt: the system messages replayed in order (later ones replace sections by name). */
function systemPrompt(messages: readonly Message[]): string {
	const base: string[] = [];
	const sections = new Map<string, string>();
	for (const message of messages) {
		if (message.role !== "system") continue;
		const text = textOf(message.content);
		if (text) base.push(text);
		for (const [key, value] of Object.entries(message.sections ?? {})) {
			if (value === null) sections.delete(key);
			else sections.set(key, value);
		}
	}
	return [...base, ...sections.values()].join("\n\n");
}

/** The newest id of a created or read task or page in the tool results, for "it". */
function lastId(messages: readonly Message[], kind: "task" | "page"): string | undefined {
	for (let index = messages.length - 1; index >= 0; index--) {
		const message = messages[index];
		if (message.role !== "toolResult" || !message.toolName?.includes(kind)) continue;
		const match = /"id":\s*"([0-9a-f-]{36})"/.exec(textOf(message.content));
		if (match) return match[1];
	}
	return undefined;
}

/** The tool call that the newest instruction asks for, or `undefined` when it asks for none. */
export function plan(instruction: string, messages: readonly Message[]): { name: string; args: { [key: string]: string } } | undefined {
	const target = (value: string, kind: "task" | "page"): string => (/^(it|that)$/i.test(value) ? lastId(messages, kind) : value) ?? value;
	let match: RegExpExecArray | null;
	if ((match = /create (?:a )?task "([^"]+)"/i.exec(instruction))) return { name: "create_task", args: { title: match[1] } };
	if ((match = /rename task (\S+) to "([^"]+)"/i.exec(instruction))) return { name: "update_task", args: { task_id: target(match[1], "task"), title: match[2] } };
	if ((match = /move task (\S+) to ([\w ]+?)\.?$/i.exec(instruction))) return { name: "update_task", args: { task_id: target(match[1], "task"), status: match[2].trim() } };
	if ((match = /create (?:a )?(?:page|doc|file) "([^"]+)"(?: with "([^"]*)")?/i.exec(instruction)))
		return { name: "create_page", args: { title: match[1], markdown: match[2] ?? "" } };
	if ((match = /append "([^"]+)" to (?:page|doc|file) (\S+)/i.exec(instruction)))
		return { name: "update_page", args: { page_id: target(match[2], "page"), markdown: match[1], mode: "append" } };
	if ((match = /read (?:page|doc|file) (\S+)/i.exec(instruction))) return { name: "read_page", args: { page_id: target(match[1], "page") } };
	if ((match = /search tasks "([^"]+)"/i.exec(instruction))) return { name: "search_tasks", args: { query: match[1] } };
	if (/list projects/i.test(instruction)) return { name: "list_projects", args: {} };
	if (/read messages/i.test(instruction)) return { name: "read_messages", args: {} };
	if ((match = /ask @([\w .-]+?) "([^"]+)"/i.exec(instruction))) return { name: "ask_teammate", args: { person: match[1], question: match[2] } };
	return undefined;
}

/** One scripted answer for the transcript so far. */
export function respond(messages: readonly Message[]): AssistantMessage {
	if (process.env.AGENT_MOCK_DEBUG) console.log("mock model sees", JSON.stringify(messages.map((m) => ({ role: m.role, tool: m.toolName, text: textOf(m.content).slice(0, 120), raw: JSON.stringify(m).slice(0, 700) }))));
	const last = messages[messages.length - 1];
	if (last?.role === "toolResult") {
		const result = textOf(last.content).replace(/<\/?orbit-data[^>]*>/g, "").trim();
		const short = result.length > 900 ? `${result.slice(0, 900)}…` : result;
		return fauxAssistantMessage(fauxText(last.isError ? `I could not do that. ${short}` : `Done (${last.toolName}).\n\n\`\`\`\n${short}\n\`\`\``));
	}
	const instruction = textOf(messages.filter((message) => message.role === "user").at(-1)?.content);
	if (/show (?:the )?(?:system )?prompt/i.test(instruction)) {
		return fauxAssistantMessage(fauxText(systemPrompt(messages)));
	}
	const call = plan(instruction, messages);
	if (!call) return fauxAssistantMessage(fauxText(`(mock model) I read: ${instruction.slice(0, 300)}`));
	return fauxAssistantMessage(fauxToolCall(call.name, call.args), { stopReason: "toolUse" });
}

/** A `Models` whose only model is a scripted one under the ChatGPT provider's id. */
export function mockModels(modelId: string): Models {
	const faux = fauxProvider({ provider: PROVIDER, models: [{ id: modelId, contextWindow: 200_000, maxTokens: 8_000 }], tokensPerSecond: 400 });
	const factory = (context: { messages: readonly unknown[] }) => respond(context.messages as Message[]);
	faux.setResponses(Array.from({ length: 1_000 }, () => factory));
	const models = createModels();
	models.setProvider(faux.provider);
	// The queue is long but finite: top it up.
	setInterval(() => {
		if (faux.getPendingResponseCount() < 500) faux.appendResponses(Array.from({ length: 500 }, () => factory));
	}, 10_000).unref();
	return models;
}
