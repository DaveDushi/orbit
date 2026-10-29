/**
 * When a conversation compacts. Pi compacts relative to each model's context window (near 451k of gpt-6.1-sol's
 * 500k); Orbit compacts at a fixed size, `AGENT_COMPACT_TOKENS`, so a DM with the agent that never ends does not
 * fill the window. Reasons: .ai/agent-design.md, "Compaction".
 */

import { DEFAULT_COMPACTION_POLICY, type CompactionPolicy, type CompactionSettings } from "@earendil-works/pi-durable";

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
