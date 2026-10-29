import assert from "node:assert/strict";
import { test } from "node:test";

import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import { createModels, fauxAssistantMessage, fauxProvider, fauxText, type AssistantMessage, type Message, type Models } from "@earendil-works/pi-ai";
import { DEFAULT_COMPACTION_POLICY, type HookApi } from "@earendil-works/pi-durable";

import { compactionHook, compactionSettings, summarize, type CompactWith } from "../src/compaction.ts";
import { loadConfig } from "../src/config.ts";
import { PROVIDER, routedModels } from "../src/models.ts";
import { orbitExtension } from "../src/tools.ts";

const sol = { provider: "openai-codex", id: "gpt-6.1-sol", contextWindow: 500_000 };
const small = { provider: "openai-codex", id: "gpt-6-sol", contextWindow: 272_000 };

/** Pi's thresholds for a window under a policy (pi-durable generation.js, `thresholdCompaction`). */
function thresholds(contextWindow: number, policy: { reserveTokens: number; backgroundTokens: number }) {
	const blocking = contextWindow - policy.reserveTokens;
	return { background: blocking - policy.backgroundTokens, blocking };
}

test("a 500k window compacts in the background at 250k and blocks a little above", () => {
	const override = compactionSettings([sol, small], 250_000).modelOverrides?.["openai-codex/gpt-6.1-sol"];
	assert.ok(override);
	const { background, blocking } = thresholds(sol.contextWindow, { ...DEFAULT_COMPACTION_POLICY, ...override });
	assert.equal(background, 250_000);
	assert.equal(blocking, 270_000);
	assert.equal(override.keepRecentTokens, 40_000);
});

test("a window whose default already compacts below the threshold keeps Pi's defaults", () => {
	// 272k − 16384 − 32768 = 222848 < 250000.
	assert.deepEqual(compactionSettings([small], 250_000), { modelOverrides: {} });
	assert.deepEqual(compactionSettings([{ ...sol, contextWindow: 299_152 }], 250_000), { modelOverrides: {} });
});

test("a small threshold keeps at most half of it verbatim", () => {
	assert.equal(compactionSettings([sol], 60_000).modelOverrides?.["openai-codex/gpt-6.1-sol"]?.keepRecentTokens, 30_000);
});

test("0 keeps Pi's defaults for every model", () => {
	assert.deepEqual(compactionSettings([sol, small], 0), {});
});

test("AGENT_COMPACT_TOKENS: default 250000, 0 allowed, bad values rejected", () => {
	const env = { ORBIT_AGENT_SECRET: "x".repeat(32), ORBIT_URL: "http://127.0.0.1:8000" };
	assert.equal(loadConfig(env).compactTokens, 250_000);
	assert.equal(loadConfig({ ...env, AGENT_COMPACT_TOKENS: "" }).compactTokens, 250_000);
	assert.equal(loadConfig({ ...env, AGENT_COMPACT_TOKENS: "0" }).compactTokens, 0);
	assert.equal(loadConfig({ ...env, AGENT_COMPACT_TOKENS: "120000" }).compactTokens, 120_000);
	for (const bad of ["-1", "1.5", "250k", "Infinity"]) {
		assert.throws(() => loadConfig({ ...env, AGENT_COMPACT_TOKENS: bad }), /AGENT_COMPACT_TOKENS/, bad);
	}
});

test("AGENT_COMPACT_MODEL and AGENT_COMPACT_THINKING: Luna and high by default, empty = the conversation's model", () => {
	const env = { ORBIT_AGENT_SECRET: "x".repeat(32), ORBIT_URL: "http://127.0.0.1:8000" };
	assert.equal(loadConfig(env).compactModel, "gpt-6-luna");
	assert.equal(loadConfig(env).compactThinking, "high");
	assert.equal(loadConfig({ ...env, AGENT_COMPACT_MODEL: "" }).compactModel, undefined);
	assert.equal(loadConfig({ ...env, AGENT_COMPACT_MODEL: "gpt-6-sol", AGENT_COMPACT_THINKING: "medium" }).compactThinking, "medium");
	assert.throws(() => loadConfig({ ...env, AGENT_COMPACT_MODEL: "gpt-nope" }), /AGENT_COMPACT_MODEL/);
	assert.throws(() => loadConfig({ ...env, AGENT_COMPACT_THINKING: "off" }), /AGENT_COMPACT_THINKING/);
});

const LUNA: CompactWith = { model: "gpt-6-luna", thinking: "high" };
const SESSION = "provider-session-1";
const ANN = "ann";

type Seen = { payer: string; model: string; sessionId?: string; reasoning?: string };

/**
 * Production routing over fake providers: `base` has no credential, each payer has their own `Models`, and the
 * conversation's provider session belongs to Ann.
 */
function routed(answer: () => AssistantMessage, window = 272_000): { models: Models; seen: Seen[] } {
	const seen: Seen[] = [];
	const modelsOf = (payer: string) => {
		const faux = fauxProvider({ provider: PROVIDER, models: [{ id: "gpt-6-luna", contextWindow: window, maxTokens: 128_000 }] });
		faux.setResponses(
			Array.from({ length: 10 }, () => (_context: unknown, options: { sessionId?: string; reasoning?: string } | undefined, _state: unknown, model: { id: string }) => {
				seen.push({ payer, model: model.id, sessionId: options?.sessionId, reasoning: options?.reasoning });
				return answer();
			}),
		);
		const models = createModels();
		models.setProvider(faux.provider);
		return models;
	};
	const base = modelsOf("nobody");
	const people = new Map<string, Models>();
	const forPayer = (userId: string) => people.get(userId) ?? people.set(userId, modelsOf(userId)).get(userId)!;
	return { models: routedModels(base, (sessionId) => (sessionId === SESSION ? ANN : undefined), forPayer), seen };
}

/** The part of a hook's API the compaction hook uses, with an in-memory memo like Pi's (first value wins). */
function hookApi(models: Models): HookApi {
	const memos = new Map<string, unknown>();
	return {
		conversationId: 7,
		models,
		snapshot: async () => ({ sessionId: SESSION }),
		memo: async (name: string, ...rest: unknown[]) => {
			if (rest.length === 1) return memos.get(name);
			if (!memos.has(name)) memos.set(name, rest[0]);
			return memos.get(name);
		},
	} as unknown as HookApi;
}

const source: Message[] = [
	{ role: "user", content: "Instruction from Ann: plan the launch", timestamp: 1 },
	{ role: "assistant", content: [{ type: "text", text: "Planned." }], api: "x", provider: PROVIDER, model: "gpt-6.1-sol", usage: undefined as never, stopReason: "stop", timestamp: 2 } as Message,
];
const summary = () => fauxAssistantMessage(fauxText("## Goal\nLaunch"));
type BeforeCompact = (compaction: { messages: readonly Message[]; instructions?: string }, api: HookApi, context: typeof BACKGROUND_CONTEXT) => Promise<{ summary: string } | undefined>;
const beforeCompact = (compactWith: CompactWith) => (compactionHook(compactWith).handlers as { beforeCompact: BeforeCompact }).beforeCompact;

test("the summary is written by Luna with high thinking and billed to the conversation's payer", async () => {
	const { models, seen } = routed(summary);
	const result = await beforeCompact(LUNA)({ messages: source }, hookApi(models), BACKGROUND_CONTEXT);
	assert.deepEqual(result, { summary: "## Goal\nLaunch" });
	assert.deepEqual(seen, [{ payer: ANN, model: "gpt-6-luna", sessionId: SESSION, reasoning: "high" }]);
});

test("a source too large for Luna falls back to the conversation's model without a request", async () => {
	// About 25k tokens of source in a 50k window: no room for the 30k of thinking and summary.
	const { models, seen } = routed(summary, 50_000);
	const warnings: string[] = [];
	const large: Message[] = [{ role: "user", content: "word ".repeat(20_000), timestamp: 1 }];
	assert.equal(await summarize(LUNA, { messages: large }, hookApi(models), BACKGROUND_CONTEXT, (line) => warnings.push(line)), undefined);
	assert.equal(seen.length, 0);
	assert.match(warnings.join(), /does not fit gpt-6-luna/);
	// The same window takes a small source.
	assert.equal(await summarize(LUNA, { messages: source }, hookApi(models), BACKGROUND_CONTEXT), "## Goal\nLaunch");
});

test("a failed Luna request falls back, once: a hook that runs again does not ask again", async () => {
	const { models, seen } = routed(() => fauxAssistantMessage([], { stopReason: "error", errorMessage: "invalid request" }));
	const api = hookApi(models);
	const hook = beforeCompact(LUNA);
	assert.equal(await hook({ messages: source }, api, BACKGROUND_CONTEXT), undefined);
	assert.equal(await hook({ messages: source }, api, BACKGROUND_CONTEXT), undefined);
	assert.equal(seen.length, 1);
});

test("a summary is memoized: a hook that runs again after a restart reuses it", async () => {
	const { models, seen } = routed(summary);
	const api = hookApi(models);
	const hook = beforeCompact(LUNA);
	await hook({ messages: source }, api, BACKGROUND_CONTEXT);
	assert.deepEqual(await hook({ messages: source }, api, BACKGROUND_CONTEXT), { summary: "## Goal\nLaunch" });
	assert.equal(seen.length, 1);
});

test("without a compaction model, Pi summarizes with the conversation's model as before", () => {
	const deps = { orbit: undefined as never, publicUrl: "", maxToolCalls: 1, botUserId: () => undefined };
	const tasks = (extension: { hooks?: readonly { task: string }[] }) => (extension.hooks ?? []).map((registration) => registration.task);
	assert.ok(!tasks(orbitExtension(deps)).includes("pi.compaction"));
	assert.ok(tasks(orbitExtension({ ...deps, compactWith: LUNA })).includes("pi.compaction"));
});
