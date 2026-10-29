/**
 * When a conversation compacts. Pi compacts relative to each model's context window (near 451k of gpt-6.1-sol's
 * 500k); Orbit compacts at a fixed size, `AGENT_COMPACT_TOKENS`, so a DM with the agent that never ends does not
 * fill the window, and writes the summary with a cheaper model (`AGENT_COMPACT_MODEL`). Reasons:
 * .ai/agent-design.md, "Compaction".
 */

import type { Context } from "@earendil-works/chord";
import type { AssistantMessage, Message, Models, ThinkingLevel } from "@earendil-works/pi-ai";
import { estimateTextTokens } from "@earendil-works/pi-ai/utils/estimate";
import { retryAssistantCall } from "@earendil-works/pi-ai/utils/retry";
import { CompactionTask, DEFAULT_COMPACTION_POLICY, hook, ProviderDoc, type CompactionPolicy, type CompactionSettings, type HookApi } from "@earendil-works/pi-durable";

import { PROVIDER } from "./models.ts";

/** How far above the threshold a request waits for the compaction instead of letting it run in the background. */
export const BLOCKING_MARGIN = 20_000;
/** At most this much recent context stays verbatim after a compaction. */
export const KEEP_RECENT = 40_000;

/**
 * Pi compaction settings that start a background compaction above `threshold` tokens of context, for each model
 * whose own default threshold is higher. `0` keeps Pi's defaults for every model.
 */
export function compactionSettings(models: readonly { provider: string; id: string; contextWindow: number }[], threshold: number): CompactionSettings {
	if (threshold <= 0) return {};
	const { reserveTokens, backgroundTokens } = DEFAULT_COMPACTION_POLICY;
	const modelOverrides: Record<string, Partial<CompactionPolicy>> = {};
	for (const model of models) {
		if (threshold >= model.contextWindow - reserveTokens - backgroundTokens) continue;
		// Pi blocks above `contextWindow - reserveTokens` and starts in the background `backgroundTokens` below that.
		modelOverrides[`${model.provider}/${model.id}`] = {
			reserveTokens: model.contextWindow - threshold - BLOCKING_MARGIN,
			backgroundTokens: BLOCKING_MARGIN,
			keepRecentTokens: Math.min(KEEP_RECENT, Math.floor(threshold / 2)),
		};
	}
	return { modelOverrides };
}

/** The model and thinking level that write compaction summaries, instead of the conversation's model. */
export type CompactWith = { readonly model: string; readonly thinking: ThinkingLevel };

/** Room kept in the summary model's window for its thinking and the summary. */
export const SUMMARY_RESERVE = 30_000;
/** Longest tool result text the summary source keeps, as in Pi. */
const TOOL_RESULT_CHARS = 2_000;
/** Memo of the compaction task: the summary, or this mark when Pi must summarize with the conversation's model. */
const MEMO = "orbit-summary";
const FALLBACK = "\u0000fallback";

// Pi 1.4.0 does not export its summary prompt; this one asks for the same sections.
const SYSTEM = "You are a context summarization assistant. Read the conversation between a user and an AI assistant and write the summary in the format asked. Do not continue the conversation and do not answer its questions.";
const FORMAT = `Summarize the conversation above as a checkpoint that another model will use to continue the work. If it starts with an earlier summary, keep that information and add the newer messages to it.

Use this format:

## Goal
## Constraints & Preferences
## Progress
### Done
### In Progress
### Blocked
## Key Decisions
## Next Steps
## Critical Context

Be concise. Keep exact names, ids, links, numbers, and error messages. Write "(none)" for an empty section.`;

function textOf(content: Message["content"]): string {
	if (typeof content === "string") return content;
	return content.flatMap((part) => (part.type === "text" ? [part.text] : [])).join("\n");
}

/** The conversation as a plain transcript, so the summarizer reads it instead of continuing it. No system messages, no thinking. */
export function transcript(messages: readonly Message[]): string {
	const parts: string[] = [];
	for (const message of messages) {
		if (message.role === "user") {
			const text = textOf(message.content);
			if (text) parts.push(`[User]: ${text}`);
		} else if (message.role === "assistant") {
			const text = message.content.flatMap((part) => (part.type === "text" ? [part.text] : [])).join("\n");
			const calls = message.content.flatMap((part) => (part.type === "toolCall" ? [`${part.name}(${JSON.stringify(part.arguments)})`] : []));
			if (text) parts.push(`[Assistant]: ${text}`);
			if (calls.length) parts.push(`[Assistant tool calls]: ${calls.join("; ")}`);
		} else if (message.role === "toolResult") {
			const text = textOf(message.content);
			if (text) parts.push(`[Tool result]: ${text.length > TOOL_RESULT_CHARS ? `${text.slice(0, TOOL_RESULT_CHARS)}\n[... ${text.length - TOOL_RESULT_CHARS} more characters]` : text}`);
		}
	}
	return parts.join("\n\n");
}

function summaryOf(message: AssistantMessage): string | undefined {
	if (message.stopReason !== "stop" || message.content.some((part) => part.type === "toolCall")) return undefined;
	return message.content.flatMap((part) => (part.type === "text" ? [part.text] : [])).join("\n").trim() || undefined;
}

/**
 * The summary written by `compactWith`, or `undefined` when Pi must summarize with the conversation's model: the
 * source does not fit the model's window, the model is unknown, or the request fails after retries. The request
 * carries the conversation's provider session id, so `routedModels` bills it to the conversation's payer.
 */
export async function summarize(
	compactWith: CompactWith,
	compaction: { readonly messages: readonly Message[]; readonly instructions?: string },
	api: Pick<HookApi, "conversationId" | "models" | "snapshot">,
	context: Context,
	warn: (line: string) => void = console.warn,
): Promise<string | undefined> {
	const skip = (line: string) => (warn(line), undefined);
	const models: Models = api.models;
	const model = models.getModel(PROVIDER, compactWith.model);
	if (!model) return skip(`agent: compaction model ${compactWith.model} is not available; using the conversation's model`);
	const focus = compaction.instructions ? `\n\nAlso focus on: ${compaction.instructions}` : "";
	const prompt = `<conversation>\n${transcript(compaction.messages)}\n</conversation>\n\n${FORMAT}${focus}`;
	const tokens = estimateTextTokens(SYSTEM + prompt);
	if (tokens + SUMMARY_RESERVE > model.contextWindow) return skip(`agent: compaction source of ~${tokens} tokens does not fit ${compactWith.model}; using the conversation's model`);
	const sessionId = (await api.snapshot(ProviderDoc, api.conversationId, context))?.sessionId;
	if (!sessionId) return skip("agent: conversation has no provider session; using the conversation's model for compaction");
	const now = Date.now();
	const request = () =>
		models.completeSimple(
			model,
			{ messages: [{ role: "system", content: SYSTEM, timestamp: now }, { role: "user", content: [{ type: "text", text: prompt }], timestamp: now }] },
			{ sessionId, reasoning: compactWith.thinking, maxTokens: SUMMARY_RESERVE, cacheRetention: "none", signal: context.abortSignal },
		);
	const message = await retryAssistantCall(request, { enabled: true, maxRetries: 2, baseDelayMs: 2_000 }, context.abortSignal);
	context.abortSignal?.throwIfAborted();
	const summary = summaryOf(message);
	if (!summary) warn(`agent: compaction with ${compactWith.model} ended with ${message.stopReason}; using the conversation's model`);
	return summary;
}

/**
 * A Pi hook that writes compaction summaries with `compactWith`. The result (or the fallback) is memoized on the
 * compaction task, so a hook that runs again after a restart does not ask the model again.
 */
export function compactionHook(compactWith: CompactWith) {
	return hook(CompactionTask, {
		beforeCompact: async (compaction, api, context) => {
			let result = await api.memo<string>(MEMO, context);
			if (result === undefined) {
				let summary: string | undefined;
				try {
					summary = await summarize(compactWith, compaction, api, context);
				} catch (error) {
					context.abortSignal?.throwIfAborted();
					console.warn(`agent: compaction with ${compactWith.model} failed; using the conversation's model`, error instanceof Error ? error.name : "");
				}
				result = await api.memo<string>(MEMO, summary ?? FALLBACK, context);
			}
			return result === FALLBACK ? undefined : { summary: result };
		},
	});
}
