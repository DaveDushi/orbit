/**
 * Just enough of Orbit for the eval: the agent's internal API and the public routes the eval's cases reach, with one
 * small workspace. Every public call is recorded with the person whose agent session made it. Drive trash and share,
 * routines and send later answer "approval_required" until the driver's yes (`approve()`), as Orbit does.
 * Also a fake media sandbox broker (deploy/donna-sandbox) on a Unix socket: every run "succeeds" and prints a tiny JPEG.
 */

import { createServer, type Server } from "node:http";
import { createServer as createSocketServer, type Server as SocketServer } from "node:net";

import type { PendingInput } from "../../apps/agent/src/orbit.ts";

export const WS = "0199c3a0-0000-7000-8000-0000000000f0";
export const CHANNEL = "0199c3a0-0000-7000-8000-0000000000f1";
export const BOT = "0199c3a0-0000-7000-8000-0000000000b0";
export const ANN = "0199c3a0-0000-7000-8000-00000000000a";
export const BOB = "0199c3a0-0000-7000-8000-00000000000b";
export const SECRET = "eval-secret-0123456789abcdef0123456789abcdef";
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

let next = 0;
export const id = () => `0199c3a0-0000-7000-8000-${String(++next).padStart(12, "0")}`;

type Json = Record<string, any>;
export type Call = { method: string; path: string; as: string; body?: unknown };

const NOW = "2026-11-15T09:00:00.000Z";
const PROJECT = { id: "p1", key: "WEB", name: "Website", private: false };
const STATUSES = [
	{ id: "s1", name: "Todo", position: 0 },
	{ id: "s2", name: "In progress", position: 1 },
	{ id: "s3", name: "Done", position: 2 },
];
const task = (number: number, title: string, status = "s1") => ({ id: `0199c3a0-0000-7000-8000-1000000000${String(number).padStart(2, "0")}`, number, project_id: "p1", status_id: status, title, description: `About ${title.toLowerCase()}.`, priority: "medium", assignee_ids: [ANN], due_at: null, version: 1 });
const page = (n: number, title: string, markdown: string) => ({
	id: `0199c3a0-0000-7000-8000-2000000000${String(n).padStart(2, "0")}`,
	title,
	parent_id: null,
	private: false,
	version: 1,
	content: [{ id: `b${n}`, type: "paragraph", content: [{ type: "text", text: markdown }] }],
});
const drive = (n: number, name: string, mime: string) => ({ id: `0199c3a0-0000-7000-8000-3000000000${String(n).padStart(2, "0")}`, name, mime, size: 2048 + n, kind: "file" });

export class FakeOrbit {
	readonly queue: (PendingInput & { acked?: boolean })[] = [];
	readonly messages = new Map<string, { id: string; body: string; root: string | null; blocks?: unknown[] }>();
	readonly threads = new Map<string, { status: string; driver_user_id: string | null; waiting_for_user_id: string | null }>();
	readonly edits: { id: string; body: string }[] = [];
	readonly calls: Call[] = [];
	readonly tasks = [task(1, "Login bug on Safari"), task(2, "Footer layout"), task(3, "Login page copy", "s2"), task(4, "Release v2", "s3")];
	readonly pages = [
		page(1, "Onboarding guide", "Welcome. Read the handbook first."),
		page(2, "Bugs", "- Checkout button misaligned\n- Search returns stale results"),
		page(3, "Project plan", "Q4: ship v2, migrate billing."),
		page(4, "Release notes", "v1.9: small fixes."),
		page(5, "Brand", "Logo and colors."),
		page(6, "Budget", "See the sheet."),
	];
	readonly drive = [drive(1, "Q3 budget.xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"), drive(2, "logo.png", "image/png"), drive(3, "draft-v1.docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"), drive(4, "brochure.pdf", "application/pdf"), drive(5, "hero.jpg", "image/jpeg")];
	/** Approval ids the driver said yes to. */
	readonly approved = new Set<string>();
	/** Approved writes that ran. */
	readonly done: string[] = [];
	/** Approval questions Orbit was asked to make. */
	readonly approvals: string[] = [];
	server!: Server;
	url = "";

	async listen(): Promise<void> {
		this.server = createServer((request, response) => {
			let raw = "";
			request.on("data", (chunk) => (raw += chunk));
			request.on("end", async () => {
				const body = raw ? JSON.parse(raw) : undefined;
				const [status, value, type] = await this.route(request.method!, request.url!, request.headers, body);
				if (Buffer.isBuffer(value)) {
					response.writeHead(status, { "content-type": type ?? "application/octet-stream", "content-length": String(value.length) });
					response.end(value);
					return;
				}
				response.writeHead(status, { "content-type": "application/json" });
				response.end(value === undefined ? "" : JSON.stringify(value));
			});
		});
		await new Promise<void>((resolve) => this.server.listen(0, "127.0.0.1", resolve));
		this.url = `http://127.0.0.1:${(this.server.address() as { port: number }).port}`;
	}

	add(input: Partial<PendingInput> & Pick<PendingInput, "author_id" | "author_name" | "kind" | "body" | "thread_key">): string {
		const messageId = id();
		this.queue.push({
			message_id: messageId,
			workspace_id: WS,
			conversation_id: CHANNEL,
			conversation_kind: "public",
			conversation_name: "general",
			conversation_members: 4,
			root_message_id: input.thread_key,
			created_at: Date.now(),
			team_instructions: "",
			...input,
		});
		return messageId;
	}

	/** The driver said yes in the thread: Orbit lets each pending approval run once. */
	approve(): void {
		for (const approval of this.approvals) this.approved.add(approval);
	}

	/** A write that needs the driver's yes: "approval_required" until then, "ok" after it. */
	#approval(tool: string, args: Json, action: string, label: string, result: Json): Json {
		const key = `${tool}:${JSON.stringify(args)}`;
		const approvalId = `approval-${key.length}-${[...key].reduce((sum, char) => (sum * 31 + char.charCodeAt(0)) >>> 0, 7)}`;
		if (this.approved.has(approvalId)) {
			this.approved.delete(approvalId);
			this.done.push(key);
			return { status: "ok", label, result };
		}
		if (!this.approvals.includes(approvalId)) this.approvals.push(approvalId);
		return { status: "approval_required", approval_id: approvalId, action, expires_at: Date.now() + 1_800_000 };
	}

	async route(method: string, url: string, headers: Record<string, unknown>, body: any): Promise<[number, unknown, string?]> {
		const path = url.split("?")[0];
		const query = new URL(url, "http://x").searchParams;
		if (path.startsWith("/download/")) return [200, Buffer.from(TINY_JPEG, "base64"), "image/jpeg"];
		if (path === "/orbit-build.json") return [200, { contract: "test", revision: "4eb9a16edf3598b5f8ec9c0751cc25525321bd8b", csp_script_hash: "" }];
		if (path.startsWith("/api/internal/agent/")) {
			if (headers.authorization !== `Bearer ${SECRET}`) return [401, { code: "authentication_required" }];
			const rest = path.slice("/api/internal/agent".length);
			let match: RegExpExecArray | null;
			if (rest === "/inputs") {
				const pending = this.queue.filter((input) => !input.acked);
				if (!pending.length) await sleep(20);
				return [200, { bot_user_id: BOT, bot_name: "Donna", model: null, thinking: null, inputs: pending }];
			}
			if ((match = /^\/inputs\/(.+)\/ack$/.exec(rest))) {
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
				const message = [...this.messages.values()].find((candidate) => candidate.id === match![1]);
				if (message) Object.assign(message, { body: body.body, blocks: body.blocks });
				this.edits.push({ id: match[1], body: body.body });
				return [204, undefined];
			}
			if (rest === "/sessions") {
				const driving = [...this.threads.values()].some((thread) => thread.driver_user_id === body.user_id && thread.status !== "idle");
				return driving ? [200, { token: `orbit-agent.${body.user_id}`, expires_at: Date.now() + 900_000 }] : [403, { code: "agent_session_refused" }];
			}
			if (/^\/credentials\/(.+)$/.test(rest) && method === "GET") return [200, { credential: { type: "oauth", access: "a", refresh: "r", expires: Date.now() + 1e9 } }];
			if (/^\/workspaces\/.+\/people$/.test(rest)) return [200, { people: [{ id: ANN, name: "Ann" }, { id: BOB, name: "Bob" }, { id: BOT, name: "Donna" }] }];
			return [404, { code: "not_found" }];
		}
		const token = String(headers.authorization ?? "").replace(/^Bearer orbit-agent\./, "");
		const prefix = `/api/v1/workspaces/${WS}`;
		if (!path.startsWith(prefix) || !token) return [401, { code: "authentication_required" }];
		const rest = path.slice(prefix.length);
		this.calls.push({ method, path: rest, as: token, ...(body === undefined ? {} : { body }) });
		return this.publicRoute(method, rest, query, body, token);
	}

	publicRoute(method: string, rest: string, query: URLSearchParams, body: any, as: string): [number, unknown, string?] {
		let match: RegExpExecArray | null;
		const get = method === "GET";
		// Tasks
		if (rest === "/projects") return [200, { items: [PROJECT] }];
		if (rest === "/projects/p1/statuses") return [200, { items: STATUSES }];
		if (rest === "/members") return [200, { items: [{ user_id: ANN, display_name: "Ann", role: "owner" }, { user_id: BOB, display_name: "Bob", role: "member" }] }];
		if (rest === "/tasks" && get) {
			const words = (query.get("search") ?? "").toLowerCase().split(/\s+/).filter(Boolean);
			return [200, { items: this.tasks.filter((item) => words.every((word) => item.title.toLowerCase().includes(word))) }];
		}
		if (rest === "/tasks" && method === "POST") {
			// Bob may not create tasks here; Ann may.
			if (as === BOB) return [403, { code: "forbidden" }];
			const created = { ...task(this.tasks.length + 1, body.title), id: id(), description: body.description ?? "" };
			this.tasks.push(created);
			return [201, created];
		}
		if ((match = /^\/tasks\/([^/]+)$/.exec(rest))) {
			const found = this.tasks.find((item) => item.id === match![1]);
			if (!found) return [404, { code: "not_found" }];
			if (method === "PATCH") Object.assign(found, body, { version: found.version + 1 });
			return [200, found];
		}
		if (/^\/tasks\/[^/]+\/comments$/.test(rest)) return get ? [200, { items: [{ id: "c1", author_id: BOB, body: "Seen on iOS too.", created_at: NOW }], next_cursor: null }] : [201, { id: id() }];
		if (/^\/tasks\/[^/]+\/linked-docs$/.test(rest)) return get ? [200, []] : [201, { ...this.pages[2] }];
		if (/^\/tasks\/[^/]+\/followers\/me$/.test(rest)) return [200, { following: method === "PUT", user_ids: [ANN] }];
		if (rest === "/projects/p1/milestones") return [200, { items: [{ id: "m1", name: "v2 launch", target_date: "2026-12-01", task_count: 3 }] }];
		if (rest === "/projects/p1/cycles") return [200, { items: [{ id: "cy1", number: 7, name: null, starts_at: "2026-11-10T00:00:00Z", ends_at: "2026-11-24T00:00:00Z", task_count: 4 }] }];
		if (rest === "/projects/p1/label-rules" && get) return [200, { rules: [{ id: "r1", label_name: "bug", user_name: "Ann", enabled: true }], owner_labels: false, unmatched_owner_names: [] }];
		if (rest === "/projects/p1/label-rules") return [201, { id: "r2", label_name: "design", user_name: "Bob", enabled: true }];
		if (rest === "/projects/p1/labels") return [200, { items: [{ id: "l1", name: "bug" }, { id: "l2", name: "design" }] }];
		// Docs
		if (rest === "/pages" && get) return [200, { items: this.pages }];
		if (rest === "/pages" && method === "POST") {
			const created = { ...page(this.pages.length + 1, body.title, ""), id: id() };
			this.pages.push(created);
			return [201, created];
		}
		if (rest === "/pages/search") {
			const words = (query.get("q") ?? "").toLowerCase().split(/\s+/).filter(Boolean);
			const items = this.pages.filter((item) => words.some((word) => `${item.title} ${JSON.stringify(item.content)}`.toLowerCase().includes(word)));
			return [200, { items: items.map((item) => ({ ...item, snippet: item.title })) }];
		}
		if ((match = /^\/pages\/([^/]+)$/.exec(rest))) {
			const found = this.pages.find((item) => item.id === match![1]);
			if (!found) return [404, { code: "not_found" }];
			if (method === "PATCH") Object.assign(found, body.content ? { content: body.content } : {}, { version: found.version + 1 });
			return [200, found];
		}
		if (/^\/pages\/[^/]+\/files$/.test(rest)) return [200, { items: [{ id: "f1", file_name: "budget.xlsx", kind: "spreadsheet", size_bytes: 4096 }] }];
		if (/^\/pages\/[^/]+\/linked-tasks$/.test(rest)) return [200, []];
		if (/^\/pages\/[^/]+\/files\/[^/]+\/content$/.test(rest)) return [200, fileContent("budget.xlsx", { sheets: ["2026"], sheet: "2026", rows: [["Item", "EUR"], ["Hosting", "1200"], ["Ads", "800"]] })];
		if (/^\/pages\/[^/]+\/drive-files$/.test(rest)) {
			const file = this.drive[1];
			if (get) return [200, { items: [{ block_id: "b9", position: 1, node_id: file.id, status: "available", file }] }];
			return [201, { block_id: "b10", position: body.position ?? 0, workspace_id: WS, node_id: body.node_id, status: "available", file: this.drive.find((item) => item.id === body.node_id) ?? file, url: null }];
		}
		// Chat
		if (/^\/chat\/threads\/[^/]+$/.test(rest)) return [200, { root: chat("Ann", "Kickoff for v2 is Monday."), items: [chat("Bob", "I will prepare the slides.")] }];
		if (/^\/chat\/conversations\/[^/]+\/messages$/.test(rest)) return [200, { items: [chat("Ann", "Kickoff for v2 is Monday."), chat("Bob", "Invoice 1042 is paid.")] }];
		if (rest === "/chat/conversations") return [200, { items: [{ id: CHANNEL, name: "general", kind: "public" }] }];
		if (rest === "/chat/search") return [200, { items: [{ message: chat("Bob", "Invoice 1042 is paid.") }] }];
		if (/^\/chat\/files\/[^/]+\/content$/.test(rest)) return [200, fileContent("leads.csv", { rows: [["name", "city"], ["Acme", "Graz"], ["Globex", "Linz"]] })];
		if (/^\/chat\/files\/[^/]+$/.test(rest)) return [200, Buffer.from(TINY_JPEG, "base64"), "image/jpeg"];
		// Memory
		if (/^\/agent-memory\/(team|private)\/wake$/.test(rest)) return [200, { total: 1, lines: [], text: "#0 (2026-11-01, Ann) Acme renews its contract in March." }];
		if (/^\/agent-memory\/(team|private)$/.test(rest)) {
			if (get) return [200, { items: [{ position: 0, body: "Acme renews its contract in March.", author_name: "Ann", created_at: NOW }] }];
			return [200, { position: 1, due: null }];
		}
		// Agent tools
		if (rest === "/agent-tools") return [200, { tools: [] }];
		if (rest === "/agent-tools/skills") return [200, { items: [] }];
		if (rest === "/agent-tools/drive") return [200, this.#drive(body.tool, body.arguments ?? {})];
		if (rest === "/agent-tools/scheduled") return [200, this.#scheduled(body.tool, body.arguments ?? {})];
		if (/^\/routines|^\/scripts/.test(rest)) return [200, { items: [] }];
		return [404, { code: "not_found" }];
	}

	#drive(tool: string, args: Json): Json {
		const named = (nodeId: unknown) => this.drive.find((item) => item.id === nodeId);
		switch (tool) {
			case "drive_search": {
				const words = String(args.query ?? "").toLowerCase().split(/\s+/).filter(Boolean);
				return { status: "ok", label: "Drive search", result: { items: this.drive.filter((item) => words.some((word) => item.name.toLowerCase().includes(word))) } };
			}
			case "drive_list":
				return { status: "ok", label: "Drive folder", result: { items: this.drive } };
			case "drive_fetch":
				return { status: "ok", label: "Drive file", result: { file: { ...named(args.node_id), size: Buffer.from(TINY_JPEG, "base64").length }, url: `${this.url}/download/${args.node_id}` } };
			case "drive_info":
				return { status: "ok", label: "Drive file", result: named(args.node_id) ?? {} };
			case "drive_read":
				return { status: "ok", label: "Drive file", result: { file: named(args.node_id), sheets: ["Q3"], sheet: "Q3", rows: [["Line", "EUR"], ["Salaries", "90000"], ["Tools", "4000"]] } };
			case "drive_create_folder":
				return { status: "ok", label: "Drive folder", result: { id: id(), name: args.name, kind: "folder" } };
			case "drive_trash":
				return this.#approval(tool, args, `Drive: move "${named((args.node_ids as unknown[])?.[0])?.name}" to the trash`, "Drive trash", { trashed: true, node_ids: args.node_ids });
			case "drive_share":
				return this.#approval(tool, args, `Drive: make a public link to "${named(args.node_id)?.name}"`, "Drive link", { url: "https://orbit.test/s/abc" });
			default:
				return { status: "refused", detail: `the eval has no ${tool}` };
		}
	}

	#scheduled(tool: string, args: Json): Json {
		switch (tool) {
			case "schedule_routine":
				return this.#approval(tool, args, `Routine "${args.name}": ${args.schedule}`, "routine", { id: "rt1", name: args.name, schedule: args.schedule, time_zone: "Europe/Vienna", status: "active", next_run_at: 1_763_200_000_000 });
			case "send_later":
				return this.#approval(tool, args, `Send later: ${args.body ?? args.text}`, "scheduled message", { id: "sl1", send_at: args.send_at });
			case "remind_me":
				return { status: "ok", label: "reminder", result: { id: "rm1", remind_at: args.remind_at } };
			default:
				return { status: "refused", detail: `the eval has no ${tool}` };
		}
	}
}

function chat(author: string, body: string): Json {
	return { id: id(), author_id: author === "Ann" ? ANN : BOB, body, created_at: NOW, files: [] };
}

function fileContent(name: string, parts: { sheets?: string[]; sheet?: string; rows?: string[][] }): Json {
	return { file: { id: "f1", file_name: name, kind: "file", size_bytes: 4096 }, sheets: parts.sheets ?? [], sheet: parts.sheet ?? null, rows: parts.rows ?? null, row_numbers: null, text: null, next_offset: null, no_text: false };
}

/** A 1×1 JPEG. */
const TINY_JPEG =
	"/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////wgALCAABAAEBAREA/8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABPxA=";

/**
 * A fake media sandbox broker: reads the request line (and any stdin), answers with stderr "800 600 JPEG" and a tiny
 * JPEG on stdout, exit 0. It runs nothing.
 */
export async function fakeBroker(socketPath: string): Promise<SocketServer> {
	const frame = (kind: string, payload: Buffer) => {
		const head = Buffer.alloc(5);
		head.write(kind, 0, "latin1");
		head.writeUInt32BE(payload.length, 1);
		return Buffer.concat([head, payload]);
	};
	const server = createSocketServer((socket) => {
		let buffered = "";
		let answered = false;
		socket.on("data", (chunk) => {
			if (answered) return;
			buffered += chunk.toString("latin1");
			const end = buffered.indexOf("\n");
			if (end < 0) return;
			const request = JSON.parse(buffered.slice(0, end)) as { op: string; stdin?: number; script?: string };
			const answer = () => {
				answered = true;
				if (request.op === "gc") return socket.end(frame("x", Buffer.from(JSON.stringify({ removed: 0 }))));
				// A copy in prints its size (`stat -c %s`); a view prints the size to stderr and the JPEG.
				if (request.script?.includes("stat -c %s")) socket.write(frame("o", Buffer.from(`${request.stdin ?? 0}\n`)));
				else {
					socket.write(frame("e", Buffer.from("800 600 JPEG\n")));
					socket.write(frame("o", Buffer.from(TINY_JPEG, "base64")));
				}
				socket.end(frame("x", Buffer.from(JSON.stringify({ exit: 0, signal: null, timed_out: false }))));
			};
			// Wait for all of stdin before answering, as the broker does.
			const want = request.stdin ?? 0;
			let have = Buffer.byteLength(buffered.slice(end + 1), "latin1");
			if (have >= want) return answer();
			socket.removeAllListeners("data");
			socket.on("data", (more) => {
				have += more.length;
				if (have >= want && !answered) answer();
			});
		});
		socket.on("error", () => undefined);
	});
	await new Promise<void>((resolve) => server.listen(socketPath, resolve));
	return server;
}
