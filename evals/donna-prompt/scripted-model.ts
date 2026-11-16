/**
 * The eval's scripted model: a fake ChatGPT model that plays a case's steps (./cases.ts) with the tools each request
 * offers, and measures each request. Policy, the same in both modes:
 * - A step whose tools are all offered directly is one turn of direct calls (parallel calls in one turn).
 * - Otherwise the step goes into a codemode script; consecutive such steps share one script, with the ids passed on in
 *   JavaScript. Before the first script that calls a tool the model does not know yet (not declared in the codemode
 *   prompt section and not found earlier in the conversation), one find_tools turn with one call per such tool.
 * - A `bulk` case is one script in both modes (today's prompt asks for codemode for many items).
 * - Last, a turn with the answer; none after a call that ends the run (an approval, a question).
 * - After the driver's "yes", the approval step runs again (direct or script, as before), then the answer.
 *
 * Tokens are estimated as characters / 4 of what the request carries (system prompt with its sections, tool
 * definitions, transcript), as the faux provider estimates; the cached part is the prefix shared with the conversation's
 * previous request.
 */

import { createModels, type JsonObject, type Models } from "@earendil-works/pi-ai";
import { fauxAssistantMessage, fauxProvider, fauxText, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";

import { codexModels, PROVIDER } from "../../apps/agent/src/models.ts";
import { CASES, type Case, type Step } from "./cases.ts";

export type RequestStat = { caseId: string; input: number; cached: number; toolDefs: number; fixed: number; codemode: number; tools: number; output: number; turn: string };

export type Message = { role: string; content?: unknown; toolName?: string; isError?: boolean; sections?: Record<string, string | null>; toolsAdded?: { name: string }[]; toolsRemoved?: { name: string }[] };
type Turn = { kind: "direct"; step: number } | { kind: "script"; from: number; to: number } | { kind: "find"; tools: string[] } | { kind: "bulk" } | { kind: "answer" };

const tokens = (text: string) => Math.ceil(text.length / 4);

export function textOf(content: unknown): string {
	if (typeof content === "string") return content;
	if (!Array.isArray(content)) return "";
	return content.map((part: { type?: string; text?: string; name?: string; arguments?: unknown }) => (part.type === "text" ? (part.text ?? "") : part.type === "toolCall" ? `${part.name}${JSON.stringify(part.arguments)}` : "")).join("");
}

/** The request as the model reads it: system prompt and sections, tool definitions, transcript; and the offered tools. */
export function replay(messages: readonly Message[]): { prompt: string; fixed: string; toolDefs: string; offered: Set<string>; codemodeSection: string } {
	const base: string[] = [];
	const sections = new Map<string, string>();
	const tools = new Map<string, unknown>();
	const transcript: string[] = [];
	for (const message of messages) {
		if (message.role !== "system") {
			transcript.push(`${message.role}:${textOf(message.content)}`);
			continue;
		}
		const text = textOf(message.content);
		if (text) base.push(text);
		for (const [key, value] of Object.entries(message.sections ?? {})) {
			if (value === null) sections.delete(key);
			else sections.set(key, value);
		}
		for (const tool of message.toolsAdded ?? []) tools.set(tool.name, tool);
		for (const tool of message.toolsRemoved ?? []) tools.delete(tool.name);
	}
	const toolDefs = JSON.stringify([...tools.values()]);
	const fixed = [...base, ...sections.values(), toolDefs].join("\n\n");
	return { prompt: [fixed, ...transcript].join("\n\n"), fixed, toolDefs, offered: new Set(tools.keys()), codemodeSection: sections.get("codemode") ?? "" };
}

function caseOf(messages: readonly Message[]): { case: Case; afterYes: boolean; since: number } | undefined {
	for (let index = messages.length - 1; index >= 0; index--) {
		const message = messages[index];
		if (message.role !== "user") continue;
		const text = textOf(message.content);
		const tag = /\[eval:(c\d+)\]/.exec(text);
		if (tag) {
			const found = CASES.find((item) => item.id === tag[1])!;
			const lastUser = messages.findLastIndex((candidate) => candidate.role === "user");
			return { case: found, afterYes: lastUser !== index, since: lastUser };
		}
	}
	return undefined;
}

/** The turns of a case in a request that offers `offered`; `known`: tools the model can call from a script without looking. */
export function plan(item: Case, offered: Set<string>, known: (tool: string) => boolean, afterYes: boolean): Turn[] {
	if (item.bulk) return [{ kind: "bulk" }, { kind: "answer" }];
	const direct = (step: Step) => step.every((call) => offered.has(call.tool));
	const steps = afterYes ? [item.steps.length - 1] : item.steps.map((_, index) => index);
	const turns: Turn[] = [];
	for (const index of steps) {
		const last = turns.at(-1);
		if (direct(item.steps[index])) turns.push({ kind: "direct", step: index });
		else if (last?.kind === "script" && last.to === index - 1) last.to = index;
		else turns.push({ kind: "script", from: index, to: index });
	}
	const unknown = [...new Set(turns.flatMap((turn) => (turn.kind === "script" ? item.steps.slice(turn.from, turn.to + 1).flat().map((call) => call.tool) : [])))].filter((tool) => !known(tool));
	if (unknown.length) turns.splice(turns.findIndex((turn) => turn.kind === "script"), 0, { kind: "find", tools: unknown });
	if (!item.ends || afterYes) turns.push({ kind: "answer" });
	return turns;
}

/** Group 1 of `pattern` in `text`. */
function pick(text: string, pattern: string): string {
	const match = new RegExp(pattern).exec(text);
	if (!match) throw new Error(`eval: no match for ${pattern}`);
	return match[1];
}

/** `args` with each "$<step>:<regex>" read from the results of that step. */
function resolve(args: Record<string, unknown>, results: (step: number) => string): Record<string, unknown> {
	const one = (value: unknown): unknown => {
		if (typeof value === "string") {
			const ref = /^\$(\d+):([\s\S]+)$/.exec(value);
			return ref ? pick(results(Number(ref[1])), ref[2]) : value;
		}
		return Array.isArray(value) ? value.map(one) : value;
	};
	return Object.fromEntries(Object.entries(args).map(([key, value]) => [key, one(value)]));
}

/** JavaScript of a script that runs steps `from`..`to`, passing ids found in earlier steps of the same script. */
function script(item: Case, from: number, to: number, earlier: (step: number) => string): string {
	const lines = ["const pick = (text, pattern) => { const match = new RegExp(pattern).exec(text); if (!match) throw new Error('not found: ' + pattern); return match[1]; };"];
	for (let index = from; index <= to; index++) {
		const calls = item.steps[index].map((call) => {
			const parts = Object.entries(call.args).map(([key, value]) => {
				const ref = typeof value === "string" ? /^\$(\d+):([\s\S]+)$/.exec(value) : null;
				const array = Array.isArray(value) && typeof value[0] === "string" ? /^\$(\d+):([\s\S]+)$/.exec(value[0]) : null;
				const inScript = (ref ?? array) && Number((ref ?? array)![1]) >= from;
				if (ref && inScript) return `${JSON.stringify(key)}: pick(r${ref[1]}, ${JSON.stringify(ref[2])})`;
				if (array && inScript) return `${JSON.stringify(key)}: [pick(r${array[1]}, ${JSON.stringify(array[2])})]`;
				return `${JSON.stringify(key)}: ${JSON.stringify(resolve({ value }, earlier).value)}`;
			});
			return `tools.${call.tool}({ ${parts.join(", ")} })`;
		});
		lines.push(calls.length === 1 ? `const r${index} = await ${calls[0]};` : `const r${index} = (await Promise.all([${calls.join(", ")}])).join("\\n");`);
		lines.push(`text(r${index});`);
	}
	return lines.join("\n");
}

/** The model of the eval, under the ChatGPT provider's id; `stats` gets one entry per request. */
export function scriptedModels(modelId: string, stats: RequestStat[]): Models {
	const previous = new Map<string, string>();
	const respond = (messages: Message[], sessionId: string | undefined) => {
		const found = caseOf(messages);
		const { prompt, fixed, toolDefs, offered, codemodeSection } = replay(messages);
		const cachedChars = (() => {
			const before = sessionId ? previous.get(sessionId) : undefined;
			if (!before) return 0;
			let index = 0;
			while (index < before.length && index < prompt.length && before[index] === prompt[index]) index++;
			return index;
		})();
		if (sessionId) previous.set(sessionId, prompt);
		const record = (turn: string, reply: ReturnType<typeof fauxAssistantMessage>) => {
			stats.push({ caseId: found?.case.id ?? "?", input: tokens(prompt), cached: Math.floor(cachedChars / 4), toolDefs: tokens(toolDefs), fixed: tokens(fixed), codemode: tokens(codemodeSection), tools: offered.size, output: tokens(textOf(reply.content)), turn });
			return reply;
		};
		if (!found) return record("none", fauxAssistantMessage(fauxText("(eval) no case")));
		const item = found.case;
		// Tools found earlier in this conversation, and those the codemode section declares.
		// As of the start of this run, so the plan does not change while the run goes on.
		const transcript = messages.slice(0, found.since).map((message) => textOf(message.content)).join("\n");
		const known = (tool: string) => new RegExp(`\\b${tool}\\(`).test(codemodeSection) || (transcript.includes(`find_tools{`) && transcript.includes(`${tool}: `));
		const turns = plan(item, offered, known, found.afterYes);
		const since = messages.slice(found.since + 1);
		const done = since.filter((message) => message.role === "assistant").length;
		const turn = turns[done];
		if (!turn) return record("extra", fauxAssistantMessage(fauxText("Done.")));
		// The results of each step so far: the tool results that follow the turn that ran it.
		const resultsOf = (step: number) => {
			const all = messages.map((message, index) => ({ message, index }));
			// Search every assistant turn whose calls belong to `step` (direct) or include it (script), newest first.
			for (const { message, index } of all.reverse()) {
				if (message.role !== "assistant" || !Array.isArray(message.content)) continue;
				const calls = (message.content as { type?: string; name?: string; arguments?: { code?: string } }[]).filter((part) => part.type === "toolCall");
				const matches = calls.some((call) => (call.name === "codemode" ? (call.arguments?.code ?? "").includes(`const r${step} =`) : item.steps[step]?.some((wanted) => wanted.tool === call.name)));
				if (!matches) continue;
				return messages
					.slice(index + 1)
					.filter((candidate, offset, rest) => candidate.role === "toolResult" && rest.slice(0, offset).every((between) => between.role === "toolResult"))
					.map((result) => textOf(result.content))
					.join("\n");
			}
			throw new Error(`eval ${item.id}: no results of step ${step}`);
		};
		const calls = (list: { name: string; args: Record<string, unknown> }[], label: string) =>
			record(label, fauxAssistantMessage(list.map((call) => fauxToolCall(call.name, call.args as JsonObject)), { stopReason: "toolUse" }));
		switch (turn.kind) {
			case "direct":
				return calls(item.steps[turn.step].map((call) => ({ name: call.tool, args: resolve(call.args, resultsOf) })), "direct");
			case "find":
				return calls(turn.tools.map((tool) => ({ name: "find_tools", args: { query: tool.replaceAll("_", " ") } })), "find_tools");
			case "script":
				return calls([{ name: "codemode", args: { code: script(item, turn.from, turn.to, resultsOf) } }], "script");
			case "bulk":
				return calls([{ name: "codemode", args: { code: item.bulk! } }], "script");
			case "answer": {
				const last = since.filter((message) => message.role === "toolResult").at(-1);
				const failed = last?.isError ? ` (a tool failed: ${textOf(last.content).slice(0, 200)})` : "";
				return record("answer", fauxAssistantMessage(fauxText(`Done: ${item.text.replace(/\.$/, "")}.${failed}`)));
			}
		}
	};
	const ids = [...new Set([modelId, ...codexModels().map((model) => model.id)])];
	const faux = fauxProvider({ provider: PROVIDER, models: ids.map((id) => ({ id, contextWindow: 400_000, maxTokens: 8_000, reasoning: true })), tokensPerSecond: 1_000_000 });
	const factory = (context: { messages: readonly unknown[] }, options: { sessionId?: string } | undefined) => respond(context.messages as Message[], options?.sessionId);
	faux.setResponses(Array.from({ length: 2_000 }, () => factory));
	const models = createModels();
	models.setProvider(faux.provider);
	return models;
}
