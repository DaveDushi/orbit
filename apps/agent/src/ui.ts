/**
 * Cards in the agent's answers, written the way json-render's "inline" mode writes them: the answer's text, then a
 * ```spec fence of JSONL lines, each an RFC 6902 patch (`{"op":"add","path":"/elements/x","value":{…}}`) that builds the
 * spec of ./ui-catalog.ts. While the model writes, each progress edit shows the card built so far (`liveCard`, marked
 * `live`, without clicks); the answer gets the whole card once it passes `checkUiSpec`, and Orbit checks it again. A
 * card that fails leaves the text answer. Clicks on the card come back as inputs of kind "action" (`actionText`).
 */

import { applySpecStreamPatch, parseSpecStreamLine } from "@json-render/core";

import { untrusted } from "./files.ts";
import type { PendingInput } from "./orbit.ts";
import { checkUiSpec, UI_VERSION, type UiSpec } from "./ui-catalog.ts";

const FENCE_OPEN = "```spec";
const FENCE_CLOSE = "```";
const TITLE_MAX = 150;

/** An answer's text without its ```spec fences, and the spec that their patches built (`null`: none). */
export type SplitAnswer = { text: string; spec: Record<string, unknown> | null };

/**
 * Splits what the model wrote into the text and the card. `done: false` while it still writes: the last line may be cut,
 * so a cut patch is skipped and a cut fence opener is hidden. Lines that do not parse as a patch are dropped inside a
 * fence; outside one every line is text, so JSON in an ordinary code block stays.
 */
export function splitAnswer(written: string, done: boolean): SplitAnswer {
	const lines = written.split("\n");
	const cut = done ? undefined : lines.pop();
	const text: string[] = [];
	let spec: Record<string, unknown> | null = null;
	let inFence = false;
	for (const line of lines) {
		const trimmed = line.trim();
		if (!inFence && trimmed.startsWith(FENCE_OPEN)) inFence = true;
		else if (inFence && trimmed === FENCE_CLOSE) inFence = false;
		else if (!inFence) text.push(line);
		else {
			const patch = parseSpecStreamLine(trimmed);
			if (!patch) continue;
			spec ??= {};
			try {
				applySpecStreamPatch(spec, patch);
			} catch {
				// A failed "test" op: the card is checked as a whole later.
			}
		}
	}
	const opening = (line: string) => line.startsWith("`") && (FENCE_OPEN.startsWith(line) || line.startsWith(FENCE_OPEN));
	if (cut !== undefined && !inFence && !opening(cut.trim())) text.push(cut);
	return { text: text.join("\n").replace(/\n{3,}/g, "\n\n").trim(), spec };
}

/**
 * The part of a card being written that can show now: the elements reachable from `root` that exist yet, with their
 * missing children left out. `null` until that part passes the catalog (an element whose props are still being built
 * makes it fail: the caller keeps showing the last card that passed).
 */
export function liveCard(spec: Record<string, unknown> | null): UiSpec | null {
	const root = spec?.root;
	const elements = spec?.elements;
	if (typeof root !== "string" || !elements || typeof elements !== "object") return null;
	const all = elements as Record<string, unknown>;
	const shown: Record<string, unknown> = {};
	const stack = [root];
	while (stack.length) {
		const id = stack.pop()!;
		const element = all[id];
		if (shown[id] || !element || typeof element !== "object") continue;
		const children = (element as { children?: unknown }).children;
		const present = Array.isArray(children) ? children.filter((child): child is string => typeof child === "string" && Boolean(all[child]) && !shown[child] && child !== id) : [];
		shown[id] = { ...element, ...(Array.isArray(children) ? { children: present } : {}) };
		stack.push(...present);
	}
	if (!shown[root]) return null;
	const checked = checkUiSpec({ root, elements: shown });
	return checked.ok ? checked.spec : null;
}

/** The card's name in clicks and in the stored block: the root's title. */
export function cardTitle(spec: UiSpec): string {
	const title = spec.elements[spec.root]?.props.title;
	return (typeof title === "string" && title.trim() ? title.trim() : "Card").slice(0, TITLE_MAX);
}

/** The answer message's `ui` block. `live`: the model still writes it. */
export function uiBlock(spec: UiSpec, live = false): Record<string, unknown> {
	return { type: "ui", version: UI_VERSION, title: cardTitle(spec), spec, ...(live ? { live: true } : {}) };
}

/**
 * The answer's text and card when the run is done: `card` is `"invalid"` when the model wrote a card that does not pass
 * the catalog, so the answer shows the text alone.
 */
export function finalCard(written: string): { text: string; card: UiSpec | "invalid" | null } {
	const { text, spec } = splitAnswer(written, true);
	if (!spec) return { text, card: null };
	const checked = checkUiSpec(spec);
	if (!checked.ok) console.warn("agent: the card is not valid", checked.error);
	return { text, card: checked.ok ? checked.spec : "invalid" };
}

/**
 * What the model reads for a click on its card. What was clicked comes from the card Orbit stored; the form values
 * are the clicker's text, so they are framed as data.
 */
export function actionText(input: PendingInput): string {
	const action = input.action;
	if (!action) return input.body;
	const card = `your card “${action.card?.title ?? "card"}”`;
	if (action.type === "form") {
		return `Action from ${input.author_name} (the driver of this run): they sent the form “${action.label}” on ${card}. The values are what they typed:\n${untrusted("form values", action.values ?? {})}`;
	}
	const params = action.params && Object.keys(action.params).length ? ` with params ${JSON.stringify(action.params)}` : "";
	return `Action from ${input.author_name} (the driver of this run): they clicked “${action.label}” on ${card}${params}. Do what that button offered.`;
}
