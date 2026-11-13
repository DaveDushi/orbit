/**
 * AGENT_MOCK=1: a scripted model for tests and demos without a ChatGPT account. It reads the newest instruction,
 * calls one real Orbit tool for it, and then reports the tool's result. Everything else (Pi Durable, the tools,
 * Orbit's permissions, chat) is the real code path.
 *
 * Understood instructions (quotes required around titles and text):
 *   create task "Title"            rename task <id|it> to "Title"       move task <id|it> to <Status>
 *   create page "Title" with "Text"   append "Text" to page <id|it>       read page <id|it>
 *   link page <id|it> to task <id|it>     unlink page <id|it> from task <id|it>
 *   search tasks "words"            list projects                         read messages
 *   ask @Name "Question"           show prompt (answers with the system prompt, to check names and team instructions)
 *   show model (answers with the run's model and thinking level)
 *   what's new (answers with the version section of the system prompt)
 *   remember "Fact"                 remember privately "Fact"             recall "words"
 *   recall privately "words"        what do you remember about "word"
 *   use <integration>__<tool> with {"json": "arguments"}
 *   read the files (reads every file sent with the message, one read_chat_file call each, then reports them all)
 *   find drive file "words"        read drive file <id|it>              create folder "Name"
 *   trash drive file <id|it>       share drive file <id|it>
 *   insert drive file <id|it> into page <id|it|this> [at <n>]     list drive files of page <id|it|this>
 *   which pages show drive file <id|it>     replace drive file block <block> on page <id|it> with <id|it>
 *   remove drive file block <block> from page <id|it>
 *   show my open tasks as a table (searches tasks, then answers with text and a ```spec card: table row by row, bar
 *     chart by status, buttons and a "New task" form; sending that form creates the task)
 *   schedule routine "Name" at "cron" to "Instruction" [with script "Script"] [in #channel]     list routines
 *   run code: <JavaScript>          save script "Name" with code: <JavaScript>          list scripts
 *   test routine "Name"            approve script of routine "Name"
 *   (a routine run of a saved script calls run_saved_script; when the script fails and the run may repair it, a
 *     routine instruction that ends with "fix it with: <JavaScript>" saves that code with repair_routine_script and
 *     runs the script again)
 *   remind me at <2026-11-11T17:00> "Text"     send later at <2026-11-11T17:00> "Text" [in #channel]
 *   (a routine's scheduled run carries its saved instruction, which these same rules read)
 *   show a broken card (answers with a card that is not in the catalog)
 *   what is this (from the Ask bar: reads the task, page or Drive folder of the page context; "it" and "this" in the
 *     other instructions also mean that item when the run has no newer one)
 *   edit video "clip.mp4" and image "photo.jpg" (media tools: finds both in Drive, copies the video to the sandbox,
 *     makes a contact sheet, looks at it and at the Drive image, trims and scales the video, halves the image, saves
 *     both as new Drive files, then reports what reached it, image blocks included)
 * AGENT_MOCK_TOKENS_PER_SECOND slows the streamed text (default 400), to watch a card build.
 *
 * A "yes" to an approval question (an answer, or the driver's next message in a DM) calls the last integration,
 * drive_trash, drive_share, schedule_routine or send_later tool again with the same arguments.
 * When a memory tool result asks for a summary, the mock writes one from the halves it was shown. A compaction gets a
 * short placeholder summary.
 */

import { createModels, type AssistantMessage, type JsonObject, type Models } from "@earendil-works/pi-ai";
import { fauxAssistantMessage, fauxProvider, fauxText, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";

import { codexModels, PROVIDER } from "./models.ts";
import { MEMORY_CHECK } from "./prompt.ts";

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

/** The newest message from a person, without the sender's time that starts it (`clockLine`). */
function newestInstruction(messages: readonly Message[]): string {
	// The memory check is Orbit's, not the driver's words: the rules read only what the driver wrote.
	return textOf(messages[lastUser(messages)]?.content).replace(/^\[[^\]\n]*\]\n/, "").replace(`\n\n${MEMORY_CHECK}`, "");
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

const MEDIA_REQUEST = /edit video "([^"]+)" and image "([^"]+)"/i;

/** The tool results of this run, oldest first. */
function runResults(messages: readonly Message[]): Message[] {
	return messages.slice(lastUser(messages)).filter((message) => message.role === "toolResult");
}

/** The next step of the media demo (MEDIA_REQUEST), or its final answer. */
export function mediaStep(instruction: string, messages: readonly Message[]): { name: string; args: { [key: string]: unknown } } | { text: string } | undefined {
	const match = MEDIA_REQUEST.exec(instruction);
	if (!match) return undefined;
	const [, video, image] = match;
	const results = runResults(messages);
	const failed = results.find((result) => result.isError);
	if (failed) return { text: `I could not finish (${failed.toolName}): ${textOf(failed.content).slice(0, 600)}` };
	const idIn = (result: Message | undefined) => /"id":\s*"([0-9a-f-]{36})"/.exec(textOf(result?.content))?.[1];
	const stem = (name: string) => name.replace(/\.[^.]+$/, "");
	const quoted = (name: string) => `'${name.replaceAll("'", "'\\''")}'`;
	switch (results.length) {
		case 0:
			return { name: "drive_search", args: { query: video } };
		case 1:
			return { name: "drive_search", args: { query: image } };
		case 2:
			return { name: "work_from_drive", args: { node_id: idIn(results[0]) } };
		case 3:
			return {
				name: "media_shell",
				args: { command: `ffprobe -v error -show_entries format=duration:stream=codec_type,width,height -of compact ${quoted(video)}; ffmpeg -v error -i ${quoted(video)} -vf "fps=1,scale=320:-2,tile=3x2" -frames:v 1 -y sheet.jpg` },
			};
		case 4:
			return { name: "view_image", args: { paths: ["sheet.jpg"], node_ids: [idIn(results[1])] } };
		case 5:
			return { name: "work_from_drive", args: { node_id: idIn(results[1]) } };
		case 6:
			return {
				name: "media_shell",
				args: {
					command: `ffmpeg -v error -ss 1 -t 3 -i ${quoted(video)} -vf scale=640:-2 -c:v libx264 -preset veryfast -an -y trimmed.mp4 && convert ${quoted(image)} -resize 50% small.jpg && identify -format "%f %wx%h\\n" small.jpg && ffprobe -v error -show_entries format=duration -of csv=p=0 trimmed.mp4`,
				},
			};
		case 7:
			return { name: "work_to_drive", args: { path: "trimmed.mp4", name: `${stem(video)} (trimmed, 640px).mp4`, on_conflict: "rename" } };
		case 8:
			return { name: "work_to_drive", args: { path: "small.jpg", name: `${stem(image)} (half size).jpg`, on_conflict: "rename" } };
		default: {
			const seen = (results[4]?.content as { type?: string; mimeType?: string; data?: string }[] | undefined)?.filter((part) => part.type === "image") ?? [];
			const sizes = [...textOf(results[4]?.content).matchAll(/"width": (\d+),\s*"height": (\d+)/g)].map((size) => `${size[1]}×${size[2]}`);
			const saved = [results[7], results[8]].map((result) => /"name":\s*"([^"]+)"/.exec(textOf(result?.content))?.[1] ?? "?");
			const probe = textOf(results[3]?.content).match(/duration=([\d.]+)/)?.[1];
			return {
				text: `(mock model) I received ${seen.length} images (${seen.map((part) => `${part.mimeType}, ${Math.round(((part.data?.length ?? 0) * 3) / 4 / 1024)} KiB`).join("; ")}): a contact sheet of the ${probe ? `${Number(probe).toFixed(1)} s ` : ""}video and the photo (originals ${sizes.join(" and ")}).\n\nSaved as new Drive files: **${saved[0]}** (seconds 1 to 4, 640 px wide, no sound) and **${saved[1]}**.`,
			};
		}
	}
}

const CARD_REQUEST = /show (?:me )?(?:my )?(?:open )?tasks as a (?:table|card|chart)/i;

/**
 * The demo answer for CARD_REQUEST, from a search_tasks result: a short text, then a ```spec fence that builds a card of
 * the open tasks (a table filled row by row, a bar chart by status, buttons, a form), as json-render's inline mode does.
 */
export function taskCard(result: string): string {
	const json = /<orbit-data[^>]*>\n([\s\S]*)\n<\/orbit-data>/.exec(result)?.[1] ?? "[]";
	const tasks = (JSON.parse(json) as { ref: string; title: string; status: string; assignees: string[] }[]).filter((task) => !/^(done|cancell?ed|duplicate)$/i.test(task.status));
	const counts = new Map<string, number>();
	for (const task of tasks) counts.set(task.status, (counts.get(task.status) ?? 0) + 1);
	const first = tasks[0]?.ref;
	const add = (path: string, value: unknown) => JSON.stringify({ op: "add", path, value });
	const patches = [
		add("/root", "card"),
		add("/elements/card", { type: "Card", props: { title: "Open tasks", description: `${tasks.length} open tasks` }, children: ["table", "chart", "buttons", "form"] }),
		add("/elements/table", {
			type: "Table",
			props: { columns: [{ key: "ref", label: "ID" }, { key: "title", label: "Title" }, { key: "status", label: "Status" }, { key: "assignees", label: "Assignees" }], rows: [] },
			children: [],
		}),
		...tasks.map((task) => add("/elements/table/props/rows/-", { ref: task.ref, title: task.title, status: task.status, assignees: task.assignees.join(", ") })),
		add("/elements/chart", {
			type: "Chart",
			props: { kind: "bar", title: "Open tasks by status", x_label: "Status", y_label: "Tasks", series: [{ name: "Tasks", points: [...counts].map(([x, y]) => ({ x, y })) }] },
			children: [],
		}),
		add("/elements/buttons", { type: "Stack", props: { direction: "row" }, children: first ? ["plan", "open"] : ["plan"] }),
		add("/elements/plan", { type: "Button", props: { label: "Plan my week", style: "primary", action: "reply", params: { plan: "week", tasks: tasks.length } }, children: [] }),
		...(first ? [add("/elements/open", { type: "Button", props: { label: `Open ${first}`, action: "open_task", params: { task: first } }, children: [] })] : []),
		add("/elements/form", {
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
			children: [],
		}),
	];
	const summary = `${tasks.length} open tasks: ${[...counts].map(([status, count]) => `${status} ${count}`).join(", ")}.`;
	return `${summary}\n\n\`\`\`spec\n${patches.join("\n")}\n\`\`\``;
}

/** The item of the Ask bar's page context in a message (`page.ts`), if any. */
export function pageItem(message: string): { kind: string; id: string } | undefined {
	const json = /<orbit-data source="page context">\n([\s\S]*?)\n<\/orbit-data>/.exec(message)?.[1];
	if (!json) return undefined;
	try {
		const item = JSON.parse(json) as { kind?: unknown; id?: unknown };
		return typeof item.kind === "string" && typeof item.id === "string" ? { kind: item.kind, id: item.id } : undefined;
	} catch {
		return undefined;
	}
}

const PAGE_KINDS = { task: "task", page: "doc", drive: "drive_folder" } as const;

/** The tool call that the newest instruction asks for, or `undefined` when it asks for none. */
export function plan(message: string, messages: readonly Message[]): { name: string; args: { [key: string]: unknown } } | undefined {
	const open = pageItem(message);
	// The page context is data: only the person's own words are the command.
	const instruction = message.replace(/\n\nSent from the Ask bar[\s\S]*$/, "");
	const fromPage = (kind: "task" | "page" | "drive") => (open?.kind === PAGE_KINDS[kind] ? open.id : undefined);
	// Code first: a script's text may contain the words of other rules.
	if (/call run_saved_script\b/.test(instruction)) return { name: "run_saved_script", args: {} };
	let code: RegExpExecArray | null;
	if ((code = /\bsave script "([^"]+)" with code:\s*([\s\S]+)$/i.exec(instruction))) return { name: "save_script", args: { name: code[1], code: code[2].trim() } };
	if ((code = /\brun code:\s*([\s\S]+)$/i.exec(instruction))) return { name: "codemode", args: { code: code[1].trim() } };
	const target = (value: string, kind: "task" | "page" | "drive"): string => (/^(it|that|this)$/i.test(value) ? (lastId(messages, kind) ?? fromPage(kind)) : value) ?? value;
	let match: RegExpExecArray | null;
	if (/what is (?:this|open|here)/i.test(instruction) && open) {
		if (open.kind === "task") return { name: "get_task", args: { task_id: open.id } };
		if (open.kind === "doc") return { name: "read_page", args: { page_id: open.id } };
		if (open.kind === "drive_folder") return { name: "drive_list", args: { folder_id: open.id } };
	}
	if (CARD_REQUEST.test(instruction)) return { name: "search_tasks", args: { limit: 50 } };
	if ((match = /sent the form “Create task”[\s\S]*"title": "([^"]+)"/.exec(instruction))) return { name: "create_task", args: { title: match[1] } };
	if ((match = /create (?:a )?task "([^"]+)"/i.exec(instruction))) return { name: "create_task", args: { title: match[1] } };
	if ((match = /rename task (\S+) to "([^"]+)"/i.exec(instruction))) return { name: "update_task", args: { task_id: target(match[1], "task"), title: match[2] } };
	if ((match = /move task (\S+) to ([\w ]+?)\.?$/i.exec(instruction))) return { name: "update_task", args: { task_id: target(match[1], "task"), status: match[2].trim() } };
	if ((match = /create (?:a )?(?:page|doc|file) "([^"]+)"(?: with "([^"]*)")?/i.exec(instruction)))
		return { name: "create_page", args: { title: match[1], markdown: match[2] ?? "" } };
	if ((match = /append "([^"]+)" to (?:page|doc|file) (\S+)/i.exec(instruction)))
		return { name: "update_page", args: { page_id: target(match[2], "page"), markdown: match[1], mode: "append" } };
	if ((match = /insert drive file (\S+) into (?:page|doc) (\S+?)(?: at (\d+))?\.?$/i.exec(instruction)))
		return { name: "insert_drive_file", args: { node_id: target(match[1], "drive"), page_id: target(match[2], "page"), ...(match[3] ? { position: Number(match[3]) } : {}) } };
	if ((match = /list (?:the )?drive files (?:of|on) (?:page|doc) (\S+?)\.?$/i.exec(instruction))) return { name: "list_page_drive_files", args: { page_id: target(match[1], "page") } };
	if ((match = /which pages show drive file (\S+?)\??$/i.exec(instruction))) return { name: "find_drive_file_pages", args: { node_id: target(match[1], "drive") } };
	if ((match = /replace drive file block (\S+) on (?:page|doc) (\S+) with (\S+?)\.?$/i.exec(instruction)))
		return { name: "replace_drive_file", args: { block_id: match[1], page_id: target(match[2], "page"), node_id: target(match[3], "drive") } };
	if ((match = /remove drive file block (\S+) from (?:page|doc) (\S+?)\.?$/i.exec(instruction)))
		return { name: "remove_drive_file", args: { block_id: match[1], page_id: target(match[2], "page") } };
	if ((match = /find drive (?:file|folder)s? "([^"]+)"/i.exec(instruction))) return { name: "drive_search", args: { query: match[1] } };
	if ((match = /read drive file (\S+)/i.exec(instruction))) return { name: "drive_read", args: { node_id: target(match[1], "drive") } };
	if ((match = /create (?:a )?(?:drive )?folder "([^"]+)"/i.exec(instruction))) return { name: "drive_create_folder", args: { name: match[1] } };
	if ((match = /trash drive (?:file|folder) (\S+)/i.exec(instruction))) return { name: "drive_trash", args: { node_id: target(match[1], "drive") } };
	if ((match = /share drive (?:file|folder) (\S+)/i.exec(instruction))) return { name: "drive_share", args: { node_id: target(match[1], "drive") } };
	if ((match = /\bunlink (?:page|doc) (\S+) from task (\S+?)\.?$/i.exec(instruction)))
		return { name: "unlink_doc_from_task", args: { page_id: target(match[1], "page"), task_id: target(match[2], "task") } };
	if ((match = /\blink (?:page|doc) (\S+) to task (\S+?)\.?$/i.exec(instruction)))
		return { name: "link_doc_to_task", args: { page_id: target(match[1], "page"), task_id: target(match[2], "task") } };
	if ((match = /read (?:page|doc|file) (\S+)/i.exec(instruction))) return { name: "read_page", args: { page_id: target(match[1], "page") } };
	if ((match = /search tasks "([^"]+)"/i.exec(instruction))) return { name: "search_tasks", args: { query: match[1] } };
	if ((match = /schedule (?:a )?routine "([^"]+)" at "([^"]+)" to "([^"]+)"(?: with script "([^"]+)")?(?: in (#[\w-]+(?:\([0-9a-f-]+\))?))?/i.exec(instruction)))
		return { name: "schedule_routine", args: { name: match[1], cron: match[2], instruction: match[3], ...(match[4] ? { script: match[4] } : {}), ...(match[5] ? { post_in: match[5] } : {}) } };
	if (/list (?:my )?routines/i.test(instruction)) return { name: "list_routines", args: {} };
	if (/list (?:my )?(?:saved )?scripts/i.test(instruction)) return { name: "list_scripts", args: {} };
	if ((match = /test (?:my )?routine "([^"]+)"/i.exec(instruction))) return { name: "test_routine", args: { routine: match[1] } };
	if ((match = /approve (?:the )?script of (?:my )?routine "([^"]+)"/i.exec(instruction))) return { name: "approve_routine_script", args: { routine: match[1] } };
	if ((match = /remind me at (\S+) "([^"]+)"/i.exec(instruction))) return { name: "remind_me", args: { at: match[1], text: match[2] } };
	if ((match = /send later at (\S+) "([^"]+)"(?: in (#[\w-]+(?:\([0-9a-f-]+\))?))?/i.exec(instruction)))
		return { name: "send_later", args: { at: match[1], text: match[2], ...(match[3] ? { post_in: match[3] } : {}) } };
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
		const again = lastCall(messages, ["use_integration", "drive_trash", "drive_share", "schedule_routine", "send_later", "approve_routine_script"]);
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
			const instruction = newestInstruction(messages);
			const next = plan(instruction, messages);
			if (next) return fauxAssistantMessage(fauxToolCall(next.name, next.args as JsonObject), { stopReason: "toolUse" });
			const reads = messages.slice(lastUser(messages)).filter((message) => message.role === "toolResult" && message.toolName === "read_chat_file");
			const parts = reads.map((message) => {
				const text = textOf(message.content).replace(/<\/?orbit-data[^>]*>/g, "").trim();
				return `\`\`\`\n${text.length > 600 ? `${text.slice(0, 600)}…` : text}\n\`\`\``;
			});
			return fauxAssistantMessage(fauxText(`Done (\`read_chat_file\` ×${reads.length}).\n\n${parts.join("\n\n")}`));
		}
		const instruction = newestInstruction(messages);
		// A routine's failed script: the fix its instruction names, then the script again.
		const fix = /fix it with:\s*([\s\S]+)$/i.exec(instruction);
		if (fix && last.toolName === "run_saved_script" && last.isError && /call repair_routine_script/.test(textOf(last.content))) {
			return fauxAssistantMessage(fauxToolCall("repair_routine_script", { code: fix[1].trim(), change: "Replaced the step that failed (mock fix)." }), { stopReason: "toolUse" });
		}
		if (last.toolName === "repair_routine_script" && !last.isError) return fauxAssistantMessage(fauxToolCall("run_saved_script", {}), { stopReason: "toolUse" });
		const media = mediaStep(instruction, messages);
		if (media) return "text" in media ? fauxAssistantMessage(fauxText(media.text)) : fauxAssistantMessage(fauxToolCall(media.name, media.args as JsonObject), { stopReason: "toolUse" });
		if (CARD_REQUEST.test(instruction) && last.toolName === "search_tasks" && !last.isError) {
			return fauxAssistantMessage(fauxText(taskCard(textOf(last.content))));
		}
		const result = textOf(last.content).replace(/<\/?orbit-data[^>]*>/g, "").trim();
		const short = result.length > 900 ? `${result.slice(0, 900)}…` : result;
		return fauxAssistantMessage(fauxText(last.isError ? `I could not do that. ${short}` : `Done (${last.toolName}).\n\n\`\`\`\n${short}\n\`\`\``));
	}
	const instruction = newestInstruction(messages);
	if (/show a broken card/i.test(instruction)) {
		return fauxAssistantMessage(fauxText(`Here is a card.\n\n\`\`\`spec\n{"op":"add","path":"/root","value":"x"}\n{"op":"add","path":"/elements/x","value":{"type":"Iframe","props":{}}}\n\`\`\``));
	}
	if (/show (?:the )?model/i.test(instruction)) {
		return fauxAssistantMessage(fauxText(`Model: ${modelId}, thinking: ${thinking ?? "default"}`));
	}
	if (/what(?:'|’)?s new/i.test(instruction)) {
		const section = systemPrompt(messages).split("\n\n").find((part) => part.startsWith("Version and what's new ("));
		return fauxAssistantMessage(fauxText(section ?? "I have no version section."));
	}
	if (/show (?:the )?(?:system )?prompt/i.test(instruction)) {
		return fauxAssistantMessage(fauxText(systemPrompt(messages)));
	}
	const asked = /what do you remember about "([^"]+)"/i.exec(instruction);
	if (asked) {
		const found = memoryLines(messages).filter((line) => line.toLowerCase().includes(asked[1].toLowerCase()));
		return fauxAssistantMessage(fauxText(found.length ? `From memory:\n${found.join("\n")}` : `I remember nothing about “${asked[1]}”.`));
	}
	const media = mediaStep(instruction, messages);
	if (media && "name" in media) return fauxAssistantMessage(fauxToolCall(media.name, media.args as JsonObject), { stopReason: "toolUse" });
	const call = plan(instruction, messages);
	if (!call) return fauxAssistantMessage(fauxText(`(mock model) I read: ${instruction.slice(0, 300)}`));
	return fauxAssistantMessage(fauxToolCall(call.name, call.args as JsonObject), { stopReason: "toolUse" });
}

/** A `Models` with scripted models under the ChatGPT provider's id: the conversation's and the compaction model. */
export function mockModels(modelId: string, compactModelId?: string): Models {
	// Every ChatGPT model too, so a model chosen in the Default model card runs.
	const ids = [...new Set([modelId, compactModelId ?? modelId, ...codexModels().map((model) => model.id)])];
	const faux = fauxProvider({ provider: PROVIDER, models: ids.map((id) => ({ id, contextWindow: 200_000, maxTokens: 8_000, reasoning: true })), tokensPerSecond: Number(process.env.AGENT_MOCK_TOKENS_PER_SECOND) || 400 });
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
