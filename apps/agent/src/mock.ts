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
 *   show model (answers with the run's model and thinking level)
 *   remember "Fact"                 remember privately "Fact"             recall "words"
 *   recall privately "words"        what do you remember about "word"
 *   use <integration>__<tool> with {"json": "arguments"}
 *   read the files (reads every file sent with the message, one read_chat_file call each, then reports them all)
 *   find drive file "words"        read drive file <id|it>              create folder "Name"
 *   trash drive file <id|it>       share drive file <id|it>
 *   show my open tasks as a table (searches tasks, then answers with a card: table, bar chart by status, buttons and a
 *     "New task" form; sending that form creates the task)
 *
 * A "yes" to an approval question (an answer, or the driver's next message in a DM) calls the last integration,
 * drive_trash or drive_share tool again with the same arguments.
 * When a memory tool result asks for a summary, the mock writes one from the halves it was shown. A compaction gets a
 * short placeholder summary.
 */

import { createModels, type AssistantMessage, type JsonObject, type Models } from "@earendil-works/pi-ai";
import { fauxAssistantMessage, fauxProvider, fauxText, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";

import { codexModels, PROVIDER } from "./models.ts";

type Message = { role: string; content?: unknown; toolName?: string; isError?: boolean; sections?: Record<string, string | null> };

/** The lines of every memory block in the system prompt, newest section last. */
function memoryLines(messages: readonly Message[]): string[] {
	// Pi sends prompt sections as system entries; only the changed ones each time.
	const system = messages.filter((message) => message.role === "system").flatMap((message) => Object.values(message.sections ?? {}).map(String));
	const blocks = system.join("\n").match(/<orbit-data source="(?:team|private) memory">\n[\s\S]*?\n<\/orbit-data>/g) ?? [];
	return blocks.flatMap((block) => block.split("\n").slice(1, -1));
}

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

/** The newest id of a created or read task, page or Drive item in the tool results, for "it". */
function lastId(messages: readonly Message[], kind: "task" | "page" | "drive"): string | undefined {
	for (let index = messages.length - 1; index >= 0; index--) {
		const message = messages[index];
		if (message.role !== "toolResult" || !message.toolName?.includes(kind)) continue;
		const match = /"id":\s*"([0-9a-f-]{36})"/.exec(textOf(message.content));
		if (match) return match[1];
	}
	return undefined;
}

/** The newest call of one of `names` that the model made, for "yes" to an approval. */
function lastCall(messages: readonly Message[], names: readonly string[]): { name: string; args: Record<string, unknown> } | undefined {
	for (let index = messages.length - 1; index >= 0; index--) {
		const message = messages[index];
		if (message.role !== "assistant" || !Array.isArray(message.content)) continue;
		const call = (message.content as { type?: string; name?: string; arguments?: Record<string, unknown> }[]).find((part) => part.type === "toolCall" && names.includes(part.name ?? ""));
		if (call?.name && call.arguments) return { name: call.name, args: call.arguments };
	}
	return undefined;
}

/** The newest user message's index. */
function lastUser(messages: readonly Message[]): number {
	return messages.findLastIndex((message) => message.role === "user");
}

/** The ids of the files sent with the newest message that this run has not read yet. */
function unreadFiles(instruction: string, messages: readonly Message[]): string[] {
	const ids = [...instruction.matchAll(/file_id ([0-9a-f-]{36})/g)].map((match) => match[1]);
	const read = new Set(
		messages
			.slice(lastUser(messages))
			.flatMap((message) => (message.role === "assistant" && Array.isArray(message.content) ? (message.content as { type?: string; name?: string; arguments?: { file_id?: string } }[]) : []))
			.filter((part) => part.type === "toolCall" && part.name === "read_chat_file")
			.map((part) => part.arguments?.file_id),
	);
	return ids.filter((id) => !read.has(id));
}

const CARD_REQUEST = /show (?:me )?(?:my )?(?:open )?tasks as a (?:table|card|chart)/i;

/** The demo card for CARD_REQUEST, from a search_tasks result: open tasks as a table, a bar chart by status, buttons, a form. */
export function taskCard(result: string): { title: string; fallback_text: string; spec: JsonObject } {
	const json = /<orbit-data[^>]*>\n([\s\S]*)\n<\/orbit-data>/.exec(result)?.[1] ?? "[]";
	const tasks = (JSON.parse(json) as { ref: string; title: string; status: string; assignees: string[] }[]).filter((task) => !/^(done|cancell?ed|duplicate)$/i.test(task.status));
	const counts = new Map<string, number>();
	for (const task of tasks) counts.set(task.status, (counts.get(task.status) ?? 0) + 1);
	const first = tasks[0]?.ref;
	const elements: JsonObject = {
		card: { type: "Card", props: { title: "Open tasks", description: `${tasks.length} open tasks` }, children: ["table", "chart", "buttons", "form"] },
		table: {
			type: "Table",
			props: {
				columns: [{ key: "ref", label: "ID" }, { key: "title", label: "Title" }, { key: "status", label: "Status" }, { key: "assignees", label: "Assignees" }],
				rows: tasks.map((task) => ({ ref: task.ref, title: task.title, status: task.status, assignees: task.assignees.join(", ") })),
			},
		},
		chart: { type: "Chart", props: { kind: "bar", title: "Open tasks by status", x_label: "Status", y_label: "Tasks", series: [{ name: "Tasks", points: [...counts].map(([x, y]) => ({ x, y })) }] } },
		buttons: { type: "Stack", props: { direction: "row" }, children: first ? ["plan", "open"] : ["plan"] },
		plan: { type: "Button", props: { label: "Plan my week", style: "primary", action: "reply", params: { plan: "week", tasks: tasks.length } } },
		...(first ? { open: { type: "Button", props: { label: `Open ${first}`, action: "open_task", params: { task: first } } } } : {}),
		form: {
			type: "Form",
			props: {
				fields: [
					{ name: "title", label: "New task", kind: "text", required: true },
					{ name: "priority", label: "Priority", kind: "select", options: ["low", "medium", "high", "urgent"] },
					{ name: "due", label: "Due", kind: "date" },
					{ name: "notify", label: "Tell the team", kind: "checkbox" },
				],
				submit_label: "Create task",
			},
		},
	};
	return { title: "Open tasks", fallback_text: `${tasks.length} open tasks: ${[...counts].map(([status, count]) => `${status} ${count}`).join(", ")}.`, spec: { root: "card", elements } };
}

/** The tool call that the newest instruction asks for, or `undefined` when it asks for none. */
export function plan(instruction: string, messages: readonly Message[]): { name: string; args: { [key: string]: unknown } } | undefined {
	const target = (value: string, kind: "task" | "page" | "drive"): string => (/^(it|that)$/i.test(value) ? lastId(messages, kind) : value) ?? value;
	let match: RegExpExecArray | null;
	if (CARD_REQUEST.test(instruction)) return { name: "search_tasks", args: { limit: 50 } };
	if (/show a broken card/i.test(instruction)) return { name: "show_ui", args: { title: "Broken", fallback_text: "Broken.", spec: { root: "x", elements: { x: { type: "Iframe", props: {} } } } } };
	if ((match = /sent the form “Create task”[\s\S]*"title": "([^"]+)"/.exec(instruction))) return { name: "create_task", args: { title: match[1] } };
	if ((match = /create (?:a )?task "([^"]+)"/i.exec(instruction))) return { name: "create_task", args: { title: match[1] } };
	if ((match = /rename task (\S+) to "([^"]+)"/i.exec(instruction))) return { name: "update_task", args: { task_id: target(match[1], "task"), title: match[2] } };
	if ((match = /move task (\S+) to ([\w ]+?)\.?$/i.exec(instruction))) return { name: "update_task", args: { task_id: target(match[1], "task"), status: match[2].trim() } };
	if ((match = /create (?:a )?(?:page|doc|file) "([^"]+)"(?: with "([^"]*)")?/i.exec(instruction)))
		return { name: "create_page", args: { title: match[1], markdown: match[2] ?? "" } };
	if ((match = /append "([^"]+)" to (?:page|doc|file) (\S+)/i.exec(instruction)))
		return { name: "update_page", args: { page_id: target(match[2], "page"), markdown: match[1], mode: "append" } };
	if ((match = /find drive (?:file|folder)s? "([^"]+)"/i.exec(instruction))) return { name: "drive_search", args: { query: match[1] } };
	if ((match = /read drive file (\S+)/i.exec(instruction))) return { name: "drive_read", args: { node_id: target(match[1], "drive") } };
	if ((match = /create (?:a )?(?:drive )?folder "([^"]+)"/i.exec(instruction))) return { name: "drive_create_folder", args: { name: match[1] } };
	if ((match = /trash drive (?:file|folder) (\S+)/i.exec(instruction))) return { name: "drive_trash", args: { node_id: target(match[1], "drive") } };
	if ((match = /share drive (?:file|folder) (\S+)/i.exec(instruction))) return { name: "drive_share", args: { node_id: target(match[1], "drive") } };
	if ((match = /read (?:page|doc|file) (\S+)/i.exec(instruction))) return { name: "read_page", args: { page_id: target(match[1], "page") } };
	if ((match = /search tasks "([^"]+)"/i.exec(instruction))) return { name: "search_tasks", args: { query: match[1] } };
	if (/list projects/i.test(instruction)) return { name: "list_projects", args: {} };
	if (/read messages/i.test(instruction)) return { name: "read_messages", args: {} };
	if (/read (?:the )?(?:chat )?files/i.test(instruction)) {
		const [next] = unreadFiles(instruction, messages);
		return next ? { name: "read_chat_file", args: { file_id: next } } : undefined;
	}
	if ((match = /ask @([\w .-]+?) "([^"]+)"/i.exec(instruction))) return { name: "ask_teammate", args: { person: match[1], question: match[2] } };
	if ((match = /remember (privately )?"([^"]+)"/i.exec(instruction))) return { name: "memory_note", args: { scope: match[1] ? "private" : "team", text: match[2] } };
	if ((match = /recall (privately )?"([^"]+)"/i.exec(instruction))) return { name: "memory_recall", args: { scope: match[1] ? "private" : "team", query: match[2] } };
	if ((match = /use (\w+__[\w.-]+)(?: with (\{[\s\S]*\}))?\s*$/i.exec(instruction))) {
		let parsed: unknown = {};
		try {
			parsed = JSON.parse(match[2] ?? "{}");
		} catch {
			return undefined;
		}
		return { name: "use_integration", args: { tool: match[1], arguments: parsed } };
	}
	if (/^(?:Answer|Instruction) from [^\n]*\n\s*(yes|approved?)\b/i.test(instruction)) {
		const again = lastCall(messages, ["use_integration", "drive_trash", "drive_share"]);
		if (again) return again;
	}
	return undefined;
}

/** One scripted answer for the transcript so far. */
export function respond(messages: readonly Message[], modelId = "mock", thinking?: string): AssistantMessage {
	if (process.env.AGENT_MOCK_DEBUG) console.log("mock model sees", JSON.stringify(messages.map((m) => ({ role: m.role, tool: m.toolName, text: textOf(m.content).slice(0, 120), raw: JSON.stringify(m).slice(0, 700) }))));
	// Pi's compaction asks for a summary of the older conversation: never act on what that conversation said.
	if (messages[0]?.role === "system" && textOf(messages[0].content).startsWith("You are a context summarization assistant")) {
		return fauxAssistantMessage(fauxText(`## Goal\n(mock summary by ${modelId} of ${textOf(messages.at(-1)?.content).length} characters)`));
	}
	const last = messages[messages.length - 1];
	if (last?.role === "toolResult") {
		const due = /Call memory_summarize with scope "(\w+)" and block "(\d+-\d+)"\.\n<orbit-data[^>]*>\n([\s\S]*?)\n<\/orbit-data>/.exec(textOf(last.content));
		if (due) {
			const summary = `Summary: ${due[3].split("\n").map((line) => line.replace(/^#\S+ (\([^)]*\) )?/, "")).join("; ")}`;
			return fauxAssistantMessage(fauxToolCall("memory_summarize", { scope: due[1], block: due[2], text: Buffer.from(summary).subarray(0, 270).toString() }), { stopReason: "toolUse" });
		}
		if (last.toolName === "read_chat_file") {
			const instruction = textOf(messages[lastUser(messages)]?.content);
			const next = plan(instruction, messages);
			if (next) return fauxAssistantMessage(fauxToolCall(next.name, next.args as JsonObject), { stopReason: "toolUse" });
			const reads = messages.slice(lastUser(messages)).filter((message) => message.role === "toolResult" && message.toolName === "read_chat_file");
			const parts = reads.map((message) => {
				const text = textOf(message.content).replace(/<\/?orbit-data[^>]*>/g, "").trim();
				return `\`\`\`\n${text.length > 600 ? `${text.slice(0, 600)}…` : text}\n\`\`\``;
			});
			return fauxAssistantMessage(fauxText(`Done (\`read_chat_file\` ×${reads.length}).\n\n${parts.join("\n\n")}`));
		}
		const instruction = textOf(messages[lastUser(messages)]?.content);
		if (CARD_REQUEST.test(instruction) && last.toolName === "search_tasks" && !last.isError) {
			return fauxAssistantMessage(fauxToolCall("show_ui", taskCard(textOf(last.content))), { stopReason: "toolUse" });
		}
		if (last.toolName === "show_ui" && !last.isError) {
			const card = lastCall(messages, ["show_ui"]);
			return fauxAssistantMessage(fauxText(String(card?.args.fallback_text ?? "Here is the card.")));
		}
		const result = textOf(last.content).replace(/<\/?orbit-data[^>]*>/g, "").trim();
		const short = result.length > 900 ? `${result.slice(0, 900)}…` : result;
		return fauxAssistantMessage(fauxText(last.isError ? `I could not do that. ${short}` : `Done (${last.toolName}).\n\n\`\`\`\n${short}\n\`\`\``));
	}
	const instruction = textOf(messages.filter((message) => message.role === "user").at(-1)?.content);
	if (/show (?:the )?model/i.test(instruction)) {
		return fauxAssistantMessage(fauxText(`Model: ${modelId}, thinking: ${thinking ?? "default"}`));
	}
	if (/show (?:the )?(?:system )?prompt/i.test(instruction)) {
		return fauxAssistantMessage(fauxText(systemPrompt(messages)));
	}
	const asked = /what do you remember about "([^"]+)"/i.exec(instruction);
	if (asked) {
		const found = memoryLines(messages).filter((line) => line.toLowerCase().includes(asked[1].toLowerCase()));
		return fauxAssistantMessage(fauxText(found.length ? `From memory:\n${found.join("\n")}` : `I remember nothing about “${asked[1]}”.`));
	}
	const call = plan(instruction, messages);
	if (!call) return fauxAssistantMessage(fauxText(`(mock model) I read: ${instruction.slice(0, 300)}`));
	return fauxAssistantMessage(fauxToolCall(call.name, call.args as JsonObject), { stopReason: "toolUse" });
}

/** A `Models` with scripted models under the ChatGPT provider's id: the conversation's and the compaction model. */
export function mockModels(modelId: string, compactModelId?: string): Models {
	// Every ChatGPT model too, so a model chosen in Agentic → Admin runs.
	const ids = [...new Set([modelId, compactModelId ?? modelId, ...codexModels().map((model) => model.id)])];
	const faux = fauxProvider({ provider: PROVIDER, models: ids.map((id) => ({ id, contextWindow: 200_000, maxTokens: 8_000, reasoning: true })), tokensPerSecond: 400 });
	const factory = (context: { messages: readonly unknown[] }, options: { reasoning?: string } | undefined, _state: unknown, model: { id: string }) =>
		respond(context.messages as Message[], model.id, options?.reasoning);
	faux.setResponses(Array.from({ length: 1_000 }, () => factory));
	const models = createModels();
	models.setProvider(faux.provider);
	// The queue is long but finite: top it up.
	setInterval(() => {
		if (faux.getPendingResponseCount() < 500) faux.appendResponses(Array.from({ length: 500 }, () => factory));
	}, 10_000).unref();
	return models;
}
