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

const orbitData = (_method: string, path: string): unknown => {
	if (path.startsWith("/projects?")) return { items: [{ id: "p1", key: "GEN", name: "General" }] };
	if (path.startsWith("/projects/p1/statuses")) return { items: [{ id: "s1", name: "Todo", position: 0 }] };
	if (path.startsWith("/members")) return { items: [] };
	if (path === "/tasks/t1") return { id: "t1", project_id: "p1", number: 7, title: "Ship", status_id: "s1", assignee_ids: [], version: 3 };
	if (path === "/tasks/t1/linked-docs") return [{ id: "d1", title: "", icon: null, private: true, linked_at: "2026-11-14T00:00:00Z" }];
	if (path === "/pages/d1") return { id: "d1", title: "Plan", private: false, version: 1, content: [] };
	if (path === "/pages/d1/files") return { items: [] };
	if (path === "/pages/d1/linked-tasks") return [{ task: { id: "t1", project_id: "p1", number: 7, title: "Ship", status_id: "s1", version: 3 }, linked_at: "2026-11-14T00:00:00Z" }];
	return undefined;
};

test("link_doc_to_task and unlink_doc_from_task call Orbit as the driver", async () => {
	const { calls, run } = harness((method, path, body) =>
		method === "POST" ? { id: (body as { page_id: string }).page_id, title: "Plan", icon: null, private: false } : method === "DELETE" ? undefined : orbitData(method, path),
	);
	const linked = await run("link_doc_to_task", { task_id: "t1", page_id: "d1" });
	assert.equal(linked.isError, undefined);
	assert.deepEqual(data(linked.output[0].text), { task_id: "t1", id: "d1", title: "Plan", private: false, url: "https://orbit.test/docs/d1" });
	const unlinked = await run("unlink_doc_from_task", { task_id: "t1", page_id: "d1" });
	assert.equal(unlinked.isError, undefined);
	assert.deepEqual(
		calls.map((call) => [call.userId, call.method, call.path, call.body]),
		[
			["ann", "POST", "/tasks/t1/linked-docs", { page_id: "d1" }],
			["ann", "DELETE", "/tasks/t1/linked-docs/d1", undefined],
		],
	);
});

test("Orbit's refusal of a page the driver cannot see reaches the model", async () => {
	const { run } = harness(() => {
		throw new OrbitError(422, { code: "validation_failed", detail: "page_id is invalid." } as never, "POST");
	});
	const result = await run("link_doc_to_task", { task_id: "t1", page_id: "hidden" });
	assert.equal(result.isError, true);
	assert.match(result.output[0].text, /422/);
});

test("get_task lists the linked docs and read_page the linked tasks", async () => {
	const { run } = harness(orbitData);
	const task = data((await run("get_task", { task_id: "t1" })).output[0].text);
	assert.deepEqual(task.linked_docs, [{ id: "d1", title: "Untitled", private: true, url: "https://orbit.test/docs/d1" }]);
	const page = data((await run("read_page", { page_id: "d1" })).output[0].text);
	assert.deepEqual(page.linked_tasks, [{ id: "t1", ref: "GEN-7", title: "Ship", url: "https://orbit.test/tasks/t1" }]);
});
