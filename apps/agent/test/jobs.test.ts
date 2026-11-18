import assert from "node:assert/strict";
import test from "node:test";

import { approvalQuestion, detailsCard, splitAction } from "../src/integrations.ts";
import { jobReport, recordsOf } from "../src/jobs.ts";
import { checkUiSpec } from "../src/ui-catalog.ts";

test("a job's records are the rows keyed by the header; empty rows go", () => {
	assert.deepEqual(
		recordsOf([
			["Email", "Accepts Marketing", ""],
			["a@b.c", "yes", "x"],
			["", "", ""],
			["d@e.f"],
		]),
		[
			{ Email: "a@b.c", "Accepts Marketing": "yes", "column 3": "x" },
			{ Email: "d@e.f", "Accepts Marketing": "", "column 3": "" },
		],
	);
	assert.deepEqual(recordsOf([]), []);
});

test("a failed job's report says what was done, what failed and what is left", () => {
	const failed = jobReport("Import customers", false, 870, 2_224, "9 of at most 23 approved calls", "Error: email has already been taken");
	assert.equal(failed.line, "⛔ **Import customers**: stopped at 870/2,224, 1,354 left.");
	assert.match(failed.text, /after 870\/2,224 \(9 of at most 23 approved calls\); 1,354 are left/);
	assert.match(failed.text, /<orbit-data source="job error">\nError: email has already been taken/);
	assert.equal(jobReport("Import customers", true, 2_224, 2_224, "23 calls", "ok").line, "✅ **Import customers**: done, 2,224/2,224.");
});

test("a question shows the summary; the exact request is only in a closed Details card that passes the catalog", () => {
	const action = 'Shop: POST https://shop.test/graphql (GraphQL mutation: customerCreate ×100) with body\n{\n "query": "mutation { … }"\n}';
	const { summary, details } = splitAction(action);
	assert.equal(summary, "Shop: POST https://shop.test/graphql (GraphQL mutation: customerCreate ×100)");
	assert.equal(details, action);
	const question = approvalQuestion("u1", summary, "to run this outside Orbit", true);
	assert.match(question, /^<@u1> I need your approval to run this outside Orbit:\n\*\*Shop: POST/);
	assert.doesNotMatch(question, /"query"/);
	// A one-line action (Drive, routines) stays as it was: the whole action in the question.
	assert.deepEqual(splitAction("Orbit Drive: make a public link"), { summary: "Orbit Drive: make a public link", details: undefined });

	const card = detailsCard(`${"x".repeat(5_000)}\`\`\`end`);
	const checked = checkUiSpec(card);
	assert.ok(checked.ok, JSON.stringify(checked));
	assert.equal(card.elements[card.root].type, "Details");
	assert.equal(card.elements[card.root].children.length, 3);
	assert.ok(!JSON.stringify(card).includes("```end"));
	// Very long details are cut, and still pass.
	assert.ok(checkUiSpec(detailsCard("y".repeat(100_000))).ok);
});
