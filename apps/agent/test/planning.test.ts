import assert from "node:assert/strict";
import { test } from "node:test";

import { orbitExtension, ORBIT_TOOLS } from "../src/tools.ts";
import type { OrbitClient } from "../src/orbit.ts";

/** Runs a tool against a fake Orbit: the calls go through `asUser` as the driver. */
function harness(answer: (method: string, path: string, body: unknown) => unknown) {
	const calls: { method: string; path: string; body: unknown }[] = [];
	const orbit = {
		asUser: async (_userId: string, _workspaceId: string, method: string, path: string, body?: unknown) => {
			calls.push({ method, path, body });
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

const day = 24 * 60 * 60 * 1000;
const at = (offset: number) => new Date(Date.now() + offset * day).toISOString();
const cycles = [
	{ id: "c1", number: 1, name: null, starts_at: at(-20), ends_at: at(-6), task_count: 4, completed_count: 2, estimate_total: 8, estimate_completed: 3, carried_over_count: 2 },
	{ id: "c2", number: 2, name: "Launch", starts_at: at(-6), ends_at: at(8), task_count: 3, completed_count: 0, estimate_total: 5, estimate_completed: 0, carried_over_count: 0 },
	{ id: "c3", number: 3, name: null, starts_at: at(8), ends_at: at(22), task_count: 0, completed_count: 0, estimate_total: 0, estimate_completed: 0, carried_over_count: 0 },
];

const orbitData = (method: string, path: string, body: unknown): unknown => {
	if (path.startsWith("/projects?")) return { items: [{ id: "p1", key: "GEN", name: "General" }] };
	if (path.startsWith("/projects/p1/statuses")) return { items: [{ id: "s1", name: "Todo", position: 0 }] };
	if (path === "/projects/p1/milestones") return { items: [{ id: "m1", name: "Beta", target_at: null, task_count: 2, completed_count: 1, estimate_total: 3, estimate_completed: 1 }] };
	if (path === "/projects/p1/cycles") return { items: cycles };
	if (path.startsWith("/members")) return { items: [] };
	if (method === "GET" && path === "/tasks/GEN-1") return { id: "t1", project_id: "p1", status_id: "s1", number: 1, title: "A", version: 7, assignee_ids: [], milestone_id: "m1", cycle_id: "c1", estimate: 2 };
	if (method === "PATCH" || method === "POST") return { id: "t1", project_id: "p1", status_id: "s1", number: 1, title: "A", version: 8, assignee_ids: [], ...(body as object) };
	return undefined;
};

test("update_task finds the milestone by name and the cycle by current/number; null clears them", async () => {
	const { calls, run } = harness(orbitData);
	assert.equal((await run("update_task", { task_id: "GEN-1", milestone: "beta", cycle: "current", estimate: 5 })).isError, undefined);
	assert.equal((await run("update_task", { task_id: "GEN-1", cycle: "cycle 3" })).isError, undefined);
	assert.equal((await run("update_task", { task_id: "GEN-1", milestone: null, cycle: null, estimate: null })).isError, undefined);
	const patches = calls.filter((call) => call.method === "PATCH").map((call) => call.body);
	assert.deepEqual(patches, [
		{ expected_version: 7, estimate: 5, milestone_id: "m1", cycle_id: "c2" },
		{ expected_version: 7, cycle_id: "c3" },
		{ expected_version: 7, estimate: null, milestone_id: null, cycle_id: null },
	]);
});

test("an unknown milestone or cycle names the project's ones and writes nothing", async () => {
	const { calls, run } = harness(orbitData);
	await assert.rejects(run("create_task", { title: "B", project_id: "p1", cycle: "next week" }), /No cycle "next week"\. Cycles: Cycle 1, Cycle 2 \(Launch\), Cycle 3/);
	await assert.rejects(run("update_task", { task_id: "GEN-1", milestone: "GA" }), /No milestone "GA"\. Milestones: Beta/);
	assert.equal(calls.filter((call) => call.method === "POST").length, 0);
});

test("list_milestones_cycles marks the previous, current and next cycle and the carried-over tasks", async () => {
	const { run } = harness(orbitData);
	const [project] = data((await run("list_milestones_cycles", { project_id: "GEN" })).output[0].text);
	assert.deepEqual(project.milestones, [{ id: "m1", name: "Beta", target_at: null, done: "1/2", points: "1/3" }]);
	assert.deepEqual(
		project.cycles.map((cycle: any) => [cycle.name, cycle.when, cycle.done, cycle.carried_over]),
		[
			["Cycle 1", "previous", "2/4", 2],
			["Cycle 2 (Launch)", "current", "0/3", 0],
			["Cycle 3", "next", "0/0", 0],
		],
	);
});
