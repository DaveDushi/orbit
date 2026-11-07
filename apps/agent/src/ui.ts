/**
 * `show_ui`: the agent adds one card (a json-render spec of ./ui-catalog.ts) to its answer. The tool posts nothing: it
 * checks the spec, so the model can fix it in the same run, and keeps it in the thread document. The run's answer
 * message then gets the text as its body and the card as a `ui` block (`Agent.#finishRun`); Orbit checks the card
 * again. Clicks on the card come back as inputs of kind "action" (`actionText`).
 */

import type { JsonValue } from "@earendil-works/chord";
import { Type } from "@earendil-works/pi-ai";
import { defineTool } from "@earendil-works/pi-durable";

import { ThreadDoc } from "./docs.ts";
import { untrusted } from "./files.ts";
import type { PendingInput } from "./orbit.ts";
import { checkUiSpec, UI_VERSION } from "./ui-catalog.ts";

export const showUi = defineTool({
	name: "show_ui",
	description:
		"Add one card to your answer: a table, chart, task list, short form or buttons, built from the components in the Cards section. Call it at most once per answer, before your final text; a second call replaces the card. Then still write a short text answer that stands alone.",
	parameters: Type.Object({
		title: Type.String({ minLength: 1, maxLength: 150, description: "What the card shows, for example “Open tasks by status”" }),
		fallback_text: Type.String({ minLength: 1, maxLength: 2000, description: "The answer in plain text, for when the card cannot be shown (the phone)" }),
		spec: Type.Record(Type.String(), Type.Unknown(), { description: 'The json-render spec: {"root": "<id>", "elements": {…}}' }),
	}),
	execute: async (args, api, context) => {
		const checked = checkUiSpec(args.spec);
		if (!checked.ok) return { output: [{ type: "text", text: `The card is not valid. Fix it and call show_ui again:\n${checked.error}` }], isError: true };
		await api.commit(async (tx) => {
			(await tx.doc(ThreadDoc, api.conversationId)).ui = { title: args.title.trim(), fallbackText: args.fallback_text.trim(), spec: checked.spec as JsonValue };
		}, context);
		return { output: [{ type: "text", text: "The card will show under your answer. Now write the short text answer." }] };
	},
});

/** The answer message's `ui` block. */
export function uiBlock(ui: { title: string; spec: unknown }): Record<string, unknown> {
	return { type: "ui", version: UI_VERSION, title: ui.title, spec: ui.spec };
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
