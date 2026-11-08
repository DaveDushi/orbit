/** Cards written as a ```spec fence of JSONL patches: splitting them from the text, the live card, the final check. */

import assert from "node:assert/strict";
import { test } from "node:test";

import { finalCard, liveCard, splitAnswer } from "../src/ui.ts";

const patch = (path: string, value: unknown) => JSON.stringify({ op: "add", path, value });
const card = [
	patch("/root", "card"),
	patch("/elements/card", { type: "Card", props: { title: "Open tasks" }, children: ["table", "plan"] }),
	patch("/elements/table", { type: "Table", props: { columns: [{ key: "id", label: "ID" }], rows: [] }, children: [] }),
	patch("/elements/table/props/rows/-", { id: "GEN-1" }),
	patch("/elements/table/props/rows/-", { id: "GEN-2" }),
	patch("/elements/plan", { type: "Button", props: { label: "Plan", action: "reply" }, children: [] }),
];
const answer = `Two open tasks.\n\nBoth are yours.\n\n\`\`\`spec\n${card.join("\n")}\n\`\`\``;

test("the text and the card come apart; the patches build the spec", () => {
	const { text, spec } = splitAnswer(answer, true);
	assert.equal(text, "Two open tasks.\n\nBoth are yours.");
	const elements = (spec as { elements: Record<string, { props: { rows?: unknown[] } }> }).elements;
	assert.deepEqual(elements.table.props.rows, [{ id: "GEN-1" }, { id: "GEN-2" }]);
	assert.equal((spec as { root: string }).root, "card");
});

test("text after the fence stays, an ordinary code block with JSON stays, and no fence means no card", () => {
	const json = '```json\n{"op":"add","path":"/root","value":"x"}\n```';
	assert.deepEqual(splitAnswer(`Before.\n${json}\nAfter.`, true), { text: `Before.\n${json}\nAfter.`, spec: null });
	assert.equal(splitAnswer(`${answer}\n\nOne more line.`, true).text, "Two open tasks.\n\nBoth are yours.\n\nOne more line.");
	// Lines in the fence that are not patches are dropped.
	assert.deepEqual(splitAnswer('Hi\n```spec\nnot json\n{"op":"add","path":"/root","value":"a"}\n```', true), { text: "Hi", spec: { root: "a" } });
});

test("while the model writes, a cut patch and a cut fence opener are skipped, and the card shows what exists", () => {
	// Cut in the middle of the third patch: the table does not exist yet, so the card shows without it.
	const cut = answer.slice(0, answer.indexOf(card[2]) + 20);
	const { text, spec } = splitAnswer(cut, false);
	assert.equal(text, "Two open tasks.\n\nBoth are yours.");
	const live = liveCard(spec);
	assert.ok(live);
	assert.deepEqual(live.elements.card.children, []);
	assert.deepEqual(Object.keys(live.elements), ["card"]);

	// With rows in: the table shows with the rows so far.
	const rows = splitAnswer(answer.slice(0, answer.indexOf(card[4])), false);
	assert.deepEqual(liveCard(rows.spec)?.elements.table.props.rows, [{ id: "GEN-1" }]);

	// The fence opener being written is not shown as text.
	assert.equal(splitAnswer("Two open tasks.\n\n```sp", false).text, "Two open tasks.");
	assert.equal(splitAnswer("Two open tasks.\n\nStill writ", false).text, "Two open tasks.\n\nStill writ");
	// Nothing to show yet: no root, or a root that does not exist.
	assert.equal(liveCard(splitAnswer("x\n```spec\n", false).spec), null);
	assert.equal(liveCard({ root: "card", elements: {} }), null);
});

test("a live card leaves out a child that loops back and an element whose props are not valid yet", () => {
	const looped = { root: "a", elements: { a: { type: "Stack", props: {}, children: ["b"] }, b: { type: "Stack", props: {}, children: ["a"] } } };
	assert.deepEqual(liveCard(looped)?.elements.b.children, []);
	assert.equal(liveCard({ root: "a", elements: { a: { type: "Chart", props: { kind: "bar" }, children: [] } } }), null);
});

test("at the end, a card that breaks the catalog is invalid and the text stays", () => {
	const good = finalCard(answer).card;
	assert.equal(good !== "invalid" && good?.root, "card");
	const broken = finalCard(`Here.\n\`\`\`spec\n${patch("/root", "x")}\n${patch("/elements/x", { type: "Iframe", props: { src: "https://x" } })}\n\`\`\``);
	assert.deepEqual(broken, { text: "Here.", card: "invalid" });
	// A child that never came: the live card hid it, the final check refuses it.
	assert.equal(finalCard(`Hi\n\`\`\`spec\n${card.slice(0, 2).join("\n")}\n\`\`\``).card, "invalid");
	assert.deepEqual(finalCard("Just text."), { text: "Just text.", card: null });
});
