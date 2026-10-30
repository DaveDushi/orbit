/**
 * Orbit's agent sidecar. Listens on loopback for Orbit (ChatGPT sign-in), reads Orbit's queue of chat messages for
 * the agent, and runs them on Pi Durable. Configuration: docs/agent.md.
 */

import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { timingSafeEqual } from "node:crypto";

import { Agent } from "./agent.ts";
import { loadConfig } from "./config.ts";
import { Logins } from "./login.ts";
import { codexModels } from "./models.ts";
import { OrbitClient } from "./orbit.ts";

const config = loadConfig();
const orbit = new OrbitClient(config);
const agent = new Agent(config, orbit);
await agent.open();
const logins = new Logins((userId) => agent.people.of(userId), orbit, config.mock);

function authorized(request: IncomingMessage): boolean {
	const given = Buffer.from(String(request.headers["x-orbit-agent-secret"] ?? ""));
	const expected = Buffer.from(config.secret);
	return given.length === expected.length && timingSafeEqual(given, expected);
}

function send(response: ServerResponse, status: number, body: unknown): void {
	response.writeHead(status, { "content-type": "application/json" });
	response.end(JSON.stringify(body));
}

const server = createServer((request, response) => {
	const url = new URL(request.url ?? "/", "http://agent");
	if (url.pathname === "/health") return send(response, 200, { ok: true, mock: config.mock });
	if (!authorized(request)) return send(response, 401, { error: "unauthorized" });
	// Admin → AI agent: what an installation admin can choose; Orbit refuses any other model.
	if (url.pathname === "/models" && request.method === "GET") return send(response, 200, { default_model: config.model, models: codexModels() });
	const login = /^\/login\/([0-9a-f-]{36})$/.exec(url.pathname);
	if (!login) return send(response, 404, { error: "not found" });
	const userId = login[1];
	if (request.method === "GET") return send(response, 200, { login: logins.view(userId) });
	if (request.method === "DELETE") {
		logins.cancel(userId);
		return send(response, 200, { login: null });
	}
	if (request.method === "POST") {
		logins.start(userId).then(
			(view) => send(response, 200, { login: view }),
			(error: unknown) => send(response, 500, { error: String(error) }),
		);
		return;
	}
	send(response, 405, { error: "method not allowed" });
});

server.listen(config.port, config.host, () => {
	console.log(`orbit agent listening on http://${config.host}:${config.port}${config.mock ? " (mock model)" : ""}`);
});
await agent.start();

for (const signal of ["SIGINT", "SIGTERM"] as const) {
	process.once(signal, () => {
		server.close();
		void agent.stop().finally(() => process.exit(0));
	});
}
