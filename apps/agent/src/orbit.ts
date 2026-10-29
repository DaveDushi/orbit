import type { Config } from "./config.ts";

/** An Orbit answer that is not 2xx. `problem` is Orbit's Problem Details body when it sent one. */
export class OrbitError extends Error {
	readonly status: number;
	readonly problem: { code?: string; detail?: string } | undefined;
	constructor(status: number, problem: { code?: string; detail?: string } | undefined, what: string) {
		super(`${what}: ${status} ${problem?.code ?? ""} ${problem?.detail ?? ""}`.trim());
		this.status = status;
		this.problem = problem;
	}
}

export type PendingInput = {
	readonly message_id: string;
	readonly workspace_id: string;
	readonly conversation_id: string;
	readonly conversation_kind: string;
	readonly conversation_name: string;
	readonly thread_key: string;
	readonly root_message_id: string | null;
	readonly author_id: string;
	readonly author_name: string;
	readonly kind: "instruction" | "answer" | "steer";
	readonly body: string;
	readonly created_at: number;
};

export type ThreadUpdate = {
	readonly workspace_id: string;
	readonly conversation_id: string;
	readonly root_message_id: string | null;
	readonly status: "idle" | "running" | "waiting";
	readonly driver_user_id: string | null;
	readonly waiting_for_user_id: string | null;
};

const SECRET_HEADER = "x-orbit-agent-secret";

/** Orbit's internal agent API (shared secret) and its public API as one person (agent session). */
export class OrbitClient {
	readonly #config: Config;
	readonly #sessions = new Map<string, { token: string; expiresAt: number }>();

	constructor(config: Config) {
		this.#config = config;
	}

	async #fetch(method: string, path: string, headers: Record<string, string>, body: unknown, what: string): Promise<unknown> {
		const response = await fetch(`${this.#config.orbitUrl}${path}`, {
			method,
			headers: {
				...headers,
				...(this.#config.forwardedProto ? { "x-forwarded-proto": this.#config.forwardedProto } : {}),
				...(body === undefined ? {} : { "content-type": "application/json" }),
			},
			body: body === undefined ? undefined : JSON.stringify(body),
			signal: AbortSignal.timeout(40_000),
		});
		const text = await response.text();
		const parsed = text ? safeJson(text) : undefined;
		if (!response.ok) throw new OrbitError(response.status, parsed as OrbitError["problem"], what);
		return parsed;
	}

	internal(method: string, path: string, body?: unknown): Promise<unknown> {
		return this.#fetch(method, `/api/internal/agent${path}`, { [SECRET_HEADER]: this.#config.secret }, body, `${method} ${path}`);
	}

	async pendingInputs(waitSeconds: number): Promise<{ bot_user_id: string; inputs: PendingInput[] }> {
		return (await this.internal("GET", `/inputs?wait=${waitSeconds}`)) as { bot_user_id: string; inputs: PendingInput[] };
	}

	async acknowledge(messageId: string): Promise<void> {
		await this.internal("POST", `/inputs/${messageId}/ack`);
	}

	async updateThread(threadKey: string, update: ThreadUpdate): Promise<void> {
		await this.internal("PUT", `/threads/${threadKey}`, update);
	}

	/** Sends a message as the agent. The same nonce returns the first message. */
	async postMessage(message: {
		workspaceId: string;
		conversationId: string;
		rootMessageId: string | null;
		body: string;
		nonce: string;
	}): Promise<string> {
		const result = (await this.internal("POST", "/messages", {
			workspace_id: message.workspaceId,
			conversation_id: message.conversationId,
			root_message_id: message.rootMessageId,
			body: message.body,
			nonce: message.nonce,
		})) as { id: string };
		return result.id;
	}

	async editMessage(messageId: string, workspaceId: string, body: string): Promise<void> {
		await this.internal("PATCH", `/messages/${messageId}`, { workspace_id: workspaceId, body });
	}

	async readCredential(userId: string): Promise<Record<string, unknown> | undefined> {
		const result = (await this.internal("GET", `/credentials/${userId}`)) as { credential: Record<string, unknown> | null };
		return result.credential ?? undefined;
	}

	async writeCredential(userId: string, credential: Record<string, unknown>): Promise<void> {
		const accountId = typeof credential.accountId === "string" ? credential.accountId : null;
		await this.internal("PUT", `/credentials/${userId}`, { credential, account_id: accountId });
	}

	async deleteCredential(userId: string): Promise<void> {
		await this.internal("DELETE", `/credentials/${userId}`);
	}

	async invalidateCredential(userId: string): Promise<void> {
		await this.internal("POST", `/credentials/${userId}/invalid`);
	}

	async #session(userId: string, workspaceId: string, fresh: boolean): Promise<string> {
		const key = `${userId}:${workspaceId}`;
		const cached = this.#sessions.get(key);
		if (!fresh && cached && cached.expiresAt - Date.now() > 60_000) return cached.token;
		const issued = (await this.internal("POST", "/sessions", { user_id: userId, workspace_id: workspaceId })) as {
			token: string;
			expires_at: number;
		};
		this.#sessions.set(key, { token: issued.token, expiresAt: issued.expires_at });
		return issued.token;
	}

	/**
	 * Orbit's public API as `userId`, below `/api/v1/workspaces/{workspaceId}`. Everything the person cannot do
	 * fails here, exactly as in their browser.
	 */
	async asUser(userId: string, workspaceId: string, method: string, path: string, body?: unknown): Promise<unknown> {
		const call = async (fresh: boolean) =>
			this.#fetch(
				method,
				`/api/v1/workspaces/${workspaceId}${path}`,
				{ authorization: `Bearer ${await this.#session(userId, workspaceId, fresh)}` },
				body,
				`${method} ${path}`,
			);
		try {
			return await call(false);
		} catch (error) {
			// The person may have revoked the session; one more try with a new one.
			if (error instanceof OrbitError && error.status === 401) return call(true);
			throw error;
		}
	}
}

function safeJson(text: string): unknown {
	try {
		return JSON.parse(text);
	} catch {
		return { detail: text.slice(0, 200) };
	}
}
