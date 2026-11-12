import assert from "node:assert/strict";
import { test } from "node:test";

import { orbitExtension, ORBIT_TOOLS } from "../src/tools.ts";
import { OrbitError, type OrbitClient } from "../src/orbit.ts";

/** Runs a tool against a fake Orbit: the calls go through `asUser` as the driver. */
function harness(answer: (method: string, path: string, body: unknown) => unknown) {
	const calls: { userId: string; method: string; path: string; body: unknown }[] = [];
	const orbit = {
		asUser: async (userId: string, _workspaceId: string, method: string, path: string, body?: unknown) => {
			calls.push({ userId, method, path, body });
			return answer(method, path, body);
		},
	} as unknown as OrbitClient;
	orbitExtension({ orbit, publicUrl: "https://orbit.test", maxToolCalls: 10, botUserId: () => undefined });
	const api = {
		conversationId: "c1",
		callId: "call-1",
		snapshot: async () => ({ driverId: "ann", driverName: "Ann", workspaceId: "w1", conversationId: "c1", toolCalls: 0 }),
		commit: async () => undefined,
	};
	const run = (name: string, args: unknown) => {
		const tool = ORBIT_TOOLS.find((candidate) => candidate.name === name) as unknown as { execute(args: unknown, api: unknown, context: unknown): Promise<{ output: { text: string }[]; isError?: boolean }> };
		return tool.execute(args, api, {});
	};
	return { calls, run };
}

/** The JSON inside a tool's data block. */
const data = (text: string): any => JSON.parse(text.slice(text.indexOf("\n") + 1, text.lastIndexOf("\n")));

const orbitData = (method: string, path: string): unknown => {
	if (path.startsWith("/projects?")) return { items: [{ id: "p1", key: "GEN", name: "General" }] };
	if (path.startsWith("/projects/p1/labels")) return { items: [{ id: "l1", name: "Bug" }, { id: "l2", name: "owner:erol" }] };
	if (path.startsWith("/members")) return { items: [{ user_id: "u-ray", display_name: "Raymond" }] };
	if (method === "GET" && path === "/projects/p1/label-rules") return { rules: [{ id: "r1", label_name: "Bug", user_name: "Raymond", enabled: true }], owner_labels: false, unmatched_owner_names: ["nobody"] };
	return undefined;
};

test("create_label_rule finds the project, label and person by name and posts the rule as the driver", async () => {
	const { calls, run } = harness((method, path, body) =>
		method === "POST" ? { id: "r9", label_name: "Bug", user_name: "Raymond", enabled: true, ...(body as object) } : orbitData(method, path),
	);
	const result = await run("create_label_rule", { project_id: "gen", label: "bug", person: "@raymond" });
	assert.equal(result.isError, undefined);
	const post = calls.find((call) => call.method === "POST")!;
	assert.equal(post.userId, "ann");
	assert.equal(post.path, "/projects/p1/label-rules");
	assert.deepEqual(post.body, { label_id: "l1", user_id: "u-ray" });
	assert.deepEqual(data(result.output[0].text), { project: "General", id: "r9", label: "Bug", person: "Raymond", enabled: true });
});

test("Orbit's refusal of a member who is not an admin reaches the model; nothing else is asked", async () => {
	const { calls, run } = harness((method, path) => {
		if (path.includes("label-rules")) throw new OrbitError(403, { code: "task_action_forbidden", detail: "You do not have permission to change this resource." } as never, method);
		return orbitData(method, path);
	});
	const created = await run("create_label_rule", { project_id: "General", label: "Bug", person: "Raymond" });
	assert.equal(created.isError, true);
	assert.match(created.output[0].text, /Ann cannot do this.*403/);
	const listed = await run("list_label_rules", { project_id: "p1" });
	assert.equal(listed.isError, true);
	assert.equal(calls.filter((call) => call.path.includes("label-rules")).length, 2);
});

test("list_label_rules shows each rule, the owner switch and the unmatched owner names", async () => {
	const { run } = harness(orbitData);
	const listed = data((await run("list_label_rules", {})).output[0].text);
	assert.deepEqual(listed, [{ project: "General", rules: [{ id: "r1", label: "Bug", person: "Raymond", enabled: true }], owner_labels: false, unmatched_owner_names: ["nobody"] }]);
});

test("an unknown label names the project's labels", async () => {
	const { calls, run } = harness((method, path) => orbitData(method, path));
	const result = await run("create_label_rule", { project_id: "GEN", label: "feature", person: "Raymond" });
	assert.equal(result.isError, true);
	assert.match(result.output[0].text, /Labels: Bug, owner:erol/);
	assert.equal(calls.some((call) => call.method === "POST"), false);
});
