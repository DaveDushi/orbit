/**
 * The agent core against a fake Orbit: each run acts only with its driver's session, an answer continues the
 * asker's run without lending the answerer's permissions, and a run interrupted by a restart is finished by the next
 * process without doing its work twice.
 */

import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { createServer, type Server } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { Agent } from "../src/agent.ts";
import type { Config } from "../src/config.ts";
import { OrbitClient, type PendingInput } from "../src/orbit.ts";

const WS = "0199c3a0-0000-7000-8000-0000000000f0";
const CHANNEL = "0199c3a0-0000-7000-8000-0000000000f1";
const BOT = "0199c3a0-0000-7000-8000-0000000000b0";
const ANN = "0199c3a0-0000-7000-8000-00000000000a";
const BOB = "0199c3a0-0000-7000-8000-00000000000b";
const SECRET = "test-secret-0123456789abcdef0123456789abcdef";
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

let next = 0;
const id = () => `0199c3a0-0000-7000-8000-${String(++next).padStart(12, "0")}`;

/** Just enough of Orbit for the agent: the internal API and a few public routes. */
class FakeOrbit {
	readonly queue: (PendingInput & { acked?: boolean })[] = [];
	readonly messages = new Map<string, { id: string; body: string; root: string | null }>();
	readonly threads = new Map<string, { status: string; driver_user_id: string | null; waiting_for_user_id: string | null }>();
	/** Public API calls: method, path, and the person whose session made them. */
	readonly calls: { method: string; path: string; as: string }[] = [];
	readonly connected = new Set<string>([ANN, BOB]);
	failAcks = false;
	failEdits = false;
	server!: Server;
	url = "";

	async listen(): Promise<void> {
		this.server = createServer((request, response) => {
			let raw = "";
			request.on("data", (chunk) => (raw += chunk));
			request.on("end", async () => {
				const body = raw ? JSON.parse(raw) : undefined;
				const [status, value] = await this.route(request.method!, request.url!, request.headers, body);
				response.writeHead(status, { "content-type": "application/json" });
				response.end(value === undefined ? "" : JSON.stringify(value));
			});
		});
		await new Promise<void>((resolve) => this.server.listen(0, "127.0.0.1", resolve));
		const address = this.server.address() as { port: number };
		this.url = `http://127.0.0.1:${address.port}`;
	}

	add(input: Partial<PendingInput> & Pick<PendingInput, "author_id" | "author_name" | "kind" | "body" | "thread_key">): string {
		const messageId = id();
		this.queue.push({
			message_id: messageId,
			workspace_id: WS,
			conversation_id: CHANNEL,
			conversation_kind: "public",
			conversation_name: "general",
			root_message_id: input.thread_key,
			created_at: Date.now(),
			...input,
		});
		return messageId;
	}

	async route(method: string, url: string, headers: Record<string, unknown>, body: any): Promise<[number, unknown]> {
		const path = url.split("?")[0];
		if (path.startsWith("/api/internal/agent/")) {
			if (headers.authorization !== `Bearer ${SECRET}`) return [401, { code: "authentication_required" }];
			const rest = path.slice("/api/internal/agent".length);
			let match: RegExpExecArray | null;
			if (rest === "/inputs") {
				const pending = this.queue.filter((input) => !input.acked);
				if (!pending.length) await sleep(50);
				return [200, { bot_user_id: BOT, inputs: pending }];
			}
			if ((match = /^\/inputs\/(.+)\/ack$/.exec(rest))) {
				if (this.failAcks) return [503, { code: "unavailable" }];
				const input = this.queue.find((candidate) => candidate.message_id === match![1]);
				if (input) input.acked = true;
				return [204, undefined];
			}
			if ((match = /^\/threads\/(.+)$/.exec(rest))) {
				this.threads.set(match[1], body);
				return [204, undefined];
			}
			if (rest === "/messages" && method === "POST") {
				const existing = this.messages.get(body.nonce);
				if (existing) return [200, { id: existing.id }];
				const message = { id: id(), body: body.body, root: body.root_message_id };
				this.messages.set(body.nonce, message);
				return [200, { id: message.id }];
			}
			if ((match = /^\/messages\/(.+)$/.exec(rest)) && method === "PATCH") {
				if (this.failEdits && !String(body.body).startsWith("⏳")) return [503, { code: "unavailable" }];
				const message = [...this.messages.values()].find((candidate) => candidate.id === match![1]);
				if (message) message.body = body.body;
				return [204, undefined];
			}
			if (rest === "/sessions") {
				// Orbit's rule: only the driver of a running or waiting thread gets a session.
				const driving = [...this.threads.values()].some((thread) => thread.driver_user_id === body.user_id && thread.status !== "idle");
				return driving ? [200, { token: `orbit-agent.${body.user_id}`, expires_at: Date.now() + 900_000 }] : [403, { code: "agent_session_refused" }];
			}
			if ((match = /^\/credentials\/(.+)$/.exec(rest)) && method === "GET") {
				return [200, { credential: this.connected.has(match[1]) ? { type: "oauth", access: "a", refresh: "r", expires: Date.now() + 1e9 } : null }];
			}
			if (/^\/workspaces\/.+\/people$/.test(rest)) {
				return [200, { people: [{ id: ANN, name: "Ann" }, { id: BOB, name: "Bob" }, { id: BOT, name: "Orbit Agent" }] }];
			}
			return [404, { code: "not_found" }];
		}
		const token = String(headers.authorization ?? "").replace(/^Bearer orbit-agent\./, "");
		const prefix = `/api/v1/workspaces/${WS}`;
		if (!path.startsWith(prefix) || !token) return [401, { code: "authentication_required" }];
		const rest = path.slice(prefix.length);
		this.calls.push({ method, path: rest, as: token });
		if (rest === "/projects") return [200, { items: [{ id: "p1", key: "ORB", name: "General" }] }];
		if (rest === "/projects/p1/statuses") return [200, { items: [{ id: "s1", name: "Todo", position: 0 }] }];
		if (rest === "/members") return [200, { items: [{ user_id: ANN, display_name: "Ann" }, { user_id: BOB, display_name: "Bob" }] }];
		if (rest === "/tasks" && method === "POST") {
			// Bob may not create tasks here; Ann may.
			if (token === BOB) return [403, { code: "forbidden" }];
			return [201, { id: id(), number: 1, project_id: "p1", status_id: "s1", title: body.title, priority: "none", assignee_ids: [], version: 1 }];
		}
		return [404, { code: "not_found" }];
	}
}

function config(orbit: FakeOrbit, dataDir: string): Config {
	return {
		orbitUrl: orbit.url,
		publicUrl: "https://orbit.test",
		secret: SECRET,
		host: "127.0.0.1",
		port: 0,
		dataDir,
		model: "gpt-test",
		mock: true,
		forwardedProto: undefined,
		maxRunsPerUser: 3,
		maxToolCallsPerRun: 40,
	};
}

let current: FakeOrbit | undefined;

async function until(what: string, check: () => boolean, ms = 15_000): Promise<void> {
	const end = Date.now() + ms;
	while (!check()) {
		if (Date.now() > end) throw new Error(`timed out waiting for ${what}: ${JSON.stringify({ messages: [...(current?.messages.values() ?? [])], calls: current?.calls, threads: [...(current?.threads ?? [])] })}`);
		await sleep(50);
	}
}

const progressOf = (orbit: FakeOrbit, messageId: string) => orbit.messages.get(`agent:${messageId}`)?.body ?? "";

test("each run acts as its driver; an answer does not lend the answerer's permissions", async () => {
	const orbit = new FakeOrbit();
	current = orbit;
	await orbit.listen();
	const agent = new Agent(config(orbit, mkdtempSync(join(tmpdir(), "agent-"))), new OrbitClient(config(orbit, "")));
	await agent.open();
	await agent.start();
	try {
		// Ann asks for a task: every Orbit call of the run is Ann's.
		const thread = id();
		const first = orbit.add({ thread_key: thread, author_id: ANN, author_name: "Ann", kind: "instruction", body: `<@${BOT}> create task "Write docs"` });
		await until("Ann's answer", () => progressOf(orbit, first).startsWith("Done (create_task)"));
		assert.ok(orbit.calls.length > 0);
		assert.ok(orbit.calls.every((call) => call.as === ANN), JSON.stringify(orbit.calls));
		await until("the idle thread", () => orbit.threads.get(thread)?.status === "idle");

		// Ann's run asks Bob; Bob's plain reply is an answer: the run still acts as Ann.
		const asking = orbit.add({ thread_key: thread, author_id: ANN, author_name: "Ann", kind: "instruction", body: `<@${BOT}> ask @Bob "Which title?"` });
		await until("the question", () => [...orbit.messages.values()].some((message) => message.body.startsWith(`<@${BOB}> Which title?`)));
		await until("waiting", () => orbit.threads.get(thread)?.status === "waiting");
		assert.match(progressOf(orbit, asking), /Waiting for an answer from <@/);
		orbit.calls.length = 0;
		const answer = orbit.add({ thread_key: thread, author_id: BOB, author_name: "Bob", kind: "answer", body: 'create task "From Bob"' });
		await until("the answer's run", () => progressOf(orbit, answer).startsWith("Done (create_task)"));
		assert.ok(orbit.calls.every((call) => call.as === ANN), JSON.stringify(orbit.calls));
		assert.equal(orbit.threads.get(thread)?.driver_user_id, ANN);

		// Bob's own instruction runs as Bob, and Orbit refuses what Bob may not do.
		orbit.calls.length = 0;
		const bobs = orbit.add({ thread_key: thread, author_id: BOB, author_name: "Bob", kind: "instruction", body: `<@${BOT}> create task "Bob's task"` });
		await until("Bob's answer", () => progressOf(orbit, bobs).startsWith("I could not do that"));
		assert.ok(orbit.calls.every((call) => call.as === BOB), JSON.stringify(orbit.calls));
		assert.match(progressOf(orbit, bobs), /Bob cannot do this/);

		// Someone without a ChatGPT connection is asked to connect, and nothing runs.
		orbit.connected.delete(BOB);
		orbit.calls.length = 0;
		const unconnected = orbit.add({ thread_key: id(), author_id: BOB, author_name: "Bob", kind: "instruction", body: `<@${BOT}> list projects` });
		await until("the connect message", () => progressOf(orbit, unconnected).includes("connect your ChatGPT account"));
		assert.equal(orbit.calls.length, 0);
	} finally {
		await agent.stop();
		orbit.server.close();
	}
});

test("a run interrupted by a restart is finished by the next process, once", async () => {
	const orbit = new FakeOrbit();
	current = orbit;
	await orbit.listen();
	const dataDir = mkdtempSync(join(tmpdir(), "agent-"));
	const thread = id();

	// The first process runs the task, but dies before it can post the answer or acknowledge the input.
	orbit.failAcks = true;
	orbit.failEdits = true;
	const first = new Agent(config(orbit, dataDir), new OrbitClient(config(orbit, "")));
	await first.open();
	await first.start();
	const input = orbit.add({ thread_key: thread, author_id: ANN, author_name: "Ann", kind: "instruction", body: `<@${BOT}> create task "Once"` });
	await until("the task", () => orbit.calls.some((call) => call.method === "POST" && call.path === "/tasks"));
	await sleep(500);
	await first.stop();
	assert.ok(!progressOf(orbit, input).startsWith("Done"));

	// The next process finds the run, posts its answer, acknowledges the input, and does not create the task again.
	orbit.failAcks = false;
	orbit.failEdits = false;
	const second = new Agent(config(orbit, dataDir), new OrbitClient(config(orbit, "")));
	await second.open();
	await second.start();
	try {
		await until("the answer after the restart", () => progressOf(orbit, input).startsWith("Done (create_task)"));
		await until("the acknowledgement", () => orbit.queue.every((queued) => queued.acked));
		assert.equal(orbit.calls.filter((call) => call.method === "POST" && call.path === "/tasks").length, 1);
		await until("the idle thread", () => orbit.threads.get(thread)?.status === "idle");
	} finally {
		await second.stop();
		orbit.server.close();
	}
});
