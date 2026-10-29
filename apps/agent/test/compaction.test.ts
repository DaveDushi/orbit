import assert from "node:assert/strict";
import { test } from "node:test";

import { DEFAULT_COMPACTION_POLICY } from "@earendil-works/pi-durable";

import { compactionSettings } from "../src/compaction.ts";
import { loadConfig } from "../src/config.ts";

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
