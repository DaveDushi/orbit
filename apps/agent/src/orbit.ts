import type { Config } from "./config.ts";
import type { ChatFile } from "./files.ts";
import type { PageContext } from "./page.ts";

/** An Orbit answer that is not 2xx. `problem` is Orbit's Problem Details body when it sent one. */
/**
 * How long a request to Orbit may take. A tool call may wait for a token, the API, a new token and one retry, each up
 * to 30 seconds on Orbit's side, and Orbit does not time it out; the rest answer well within 40 seconds.
 */
export const requestTimeoutMs = (path: string) => (path.endsWith("/agent-tools/call") ? 130_000 : 40_000);

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
	/** People in the conversation, the agent included. */
	readonly conversation_members: number;
	readonly thread_key: string;
	readonly root_message_id: string | null;
	readonly author_id: string;
	readonly author_name: string;
	readonly kind: "instruction" | "answer" | "steer" | "action";
	readonly body: string;
	/** For kind "action": a click on the agent's card (`ui.ts`); absent from an older Orbit. */
	readonly action?: CardAction | null;
	readonly created_at: number;
	/** The workspace's team instructions; empty when it has none. */
	readonly team_instructions: string;
	/** The message's files; absent from an older Orbit. */
	readonly files?: readonly ChatFile[];
	/** Sent from the Ask bar: the page the author is on (`page.ts`); absent from Chat and an older Orbit. */
	readonly page_context?: PageContext | null;
	/** A scheduled run of the author's routine; `body` is its saved instruction. Absent from an older Orbit. */
	readonly routine?: RoutineRun | null;
	/** A run on a task, driven by the author; `body` is the comment that started it (empty when none). Absent from an older Orbit. */
	readonly task?: TaskRunInput | null;
	/** A custom agent (Settings → Agentic → Agents); `null` or absent: the default agent. */
	readonly agent?: InputAgent | null;
	/** The default agent's card in this workspace, for its inputs once set; absent from an older Orbit. */
	readonly default_agent?: DefaultAgent | null;
	/** The driver's answer level: `short`, `normal` or `thorough`; absent from an older Orbit. */
	readonly answer_level?: string;
};

/**
 * A run on a task (Orbit's `agent_task_runs`): the author assigned the task to the agent, mentioned it in a comment on
 * the task, or commented on a task assigned to it. Its progress and result go into one comment on the task, as the agent.
 */
export type TaskRunInput = {
	readonly run_id: string;
	readonly task_id: string;
	/** `ENG-12`. */
	readonly identifier: string;
	readonly title: string;
	readonly cause: "assigned" | "mentioned" | "commented";
	/** The comment that started the run. */
	readonly comment_id: string | null;
};

/** A task run that Orbit cancelled after it started (the agent was unassigned, or the task deleted): stop it. */
export type TaskRunStop = {
	readonly run_id: string;
	readonly thread_key: string;
	readonly reason: string;
};

/** A custom agent: its own bot user, name, instructions, tools and model. Orbit also enforces its tools. */
export type InputAgent = {
	readonly user_id: string;
	readonly name: string;
	readonly instructions: string;
	/** The tool names it may use (a subset of the default agent's). */
	readonly tools: readonly string[];
	/** `null`: the installation's choice. */
	readonly model: string | null;
	readonly thinking: string | null;
};

/** The default agent's own instructions and model in one workspace (Settings → Agentic → Agents). */
export type DefaultAgent = {
	readonly instructions: string;
	/** `null`: the installation's choice. */
	readonly model: string | null;
	readonly thinking: string | null;
};

/** The routine behind a scheduled run (Settings → Agentic → Routines). */
export type RoutineRun = {
	readonly id: string;
	readonly name: string;
	/** 5-field cron. */
	readonly schedule: string;
	readonly time_zone: string;
	/** A test run: in the creator's DM, nothing written. Absent from an older Orbit. */
	readonly test?: boolean;
	/** `normal`, `script` or `dry`: what the run may write (Orbit enforces it). */
	readonly writes?: string;
	/** The saved script the run runs, exactly as it was when the run started. */
	readonly script?: { readonly id: string; readonly name: string; readonly code: string; readonly hash: string } | null;
	/** The script's `store()` values from the routine's last run. */
	readonly state?: Record<string, unknown> | null;
};

/** The end of a routine run, for its history in Orbit. */
export type RunReport = {
	readonly status: "done" | "failed";
	readonly output?: string;
	readonly error?: string;
	readonly tool_calls: readonly { name: string; args: string; status: string; would_do?: string }[];
	readonly state?: Record<string, unknown> | null;
};

/** What Orbit stored for a click or form send on the agent's card: the card's own data, and the form values. */
export type CardAction = {
	readonly type: "button" | "form";
	readonly label: string;
	readonly params?: Record<string, unknown> | null;
	readonly values?: Record<string, unknown>;
	readonly card?: { readonly message_id: string; readonly title: string };
};

export type PendingInputs = {
	readonly bot_user_id: string;
	readonly bot_name: string;
	/** Chosen by an installation admin; `null` (or absent from an older Orbit): the sidecar's default. */
	readonly model?: string | null;
	readonly thinking?: string | null;
	readonly inputs: PendingInput[];
	/** Task runs to stop; absent from an older Orbit. */
	readonly stops?: readonly TaskRunStop[];
};

export type ThreadUpdate = {
	readonly workspace_id: string;
	readonly conversation_id: string;
	readonly root_message_id: string | null;
	readonly status: "idle" | "running" | "waiting";
	readonly driver_user_id: string | null;
	readonly waiting_for_user_id: string | null;
	/** The thread's custom agent; `null`: the default agent. */
	readonly agent_user_id?: string | null;
};

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
			signal: AbortSignal.timeout(requestTimeoutMs(path)),
		});
		const text = await response.text();
		const parsed = text ? safeJson(text) : undefined;
		if (!response.ok) throw new OrbitError(response.status, parsed as OrbitError["problem"], what);
		return parsed;
	}

	internal(method: string, path: string, body?: unknown): Promise<unknown> {
		// A bearer request without Origin passes Orbit's origin policy, as native clients do.
		return this.#fetch(method, `/api/internal/agent${path}`, { authorization: `Bearer ${this.#config.secret}` }, body, `${method} ${path}`);
	}

	/** The git revision of Orbit's build, from the web build manifest it serves. */
	async buildRevision(): Promise<string> {
		const manifest = (await this.#fetch("GET", "/orbit-build.json", {}, undefined, "GET /orbit-build.json")) as { revision?: unknown } | undefined;
		if (typeof manifest?.revision !== "string" || !manifest.revision) throw new Error("orbit-build.json has no revision");
		return manifest.revision;
	}

	/** The queued inputs, the bot's id and current name, and the model and thinking level of new runs (`null`: default). */
	async pendingInputs(waitSeconds: number): Promise<PendingInputs> {
		return (await this.internal("GET", `/inputs?wait=${waitSeconds}`)) as PendingInputs;
	}

	async acknowledge(messageId: string): Promise<void> {
		await this.internal("POST", `/inputs/${messageId}/ack`);
	}

	async updateThread(threadKey: string, update: ThreadUpdate): Promise<void> {
		await this.internal("PUT", `/threads/${threadKey}`, update);
	}

	/** Sends a message as the agent (`agentUserId`: a custom agent). The same nonce returns the first message. */
	async postMessage(message: {
		workspaceId: string;
		conversationId: string;
		rootMessageId: string | null;
		body: string;
		nonce: string;
		agentUserId?: string | null;
	}): Promise<string> {
		const result = (await this.internal("POST", "/messages", {
			workspace_id: message.workspaceId,
			conversation_id: message.conversationId,
			root_message_id: message.rootMessageId,
			body: message.body,
			nonce: message.nonce,
			...(message.agentUserId ? { agent_user_id: message.agentUserId } : {}),
		})) as { id: string };
		return result.id;
	}

	/** `blocks`: the answer's card (one `ui` block), which Orbit checks again. `agentUserId`: a custom agent's message. */
	async editMessage(messageId: string, workspaceId: string, body: string, blocks?: readonly unknown[], agentUserId?: string | null): Promise<void> {
		await this.internal("PATCH", `/messages/${messageId}`, { workspace_id: workspaceId, body, ...(blocks ? { blocks } : {}), ...(agentUserId ? { agent_user_id: agentUserId } : {}) });
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

	/** Writes a task run's comment on its task, as the agent: the first call makes it, later calls replace its text. */
	async writeTaskComment(runId: string, body: string): Promise<string> {
		const result = (await this.internal("PUT", `/task-runs/${runId}/comment`, { body })) as { comment_id: string };
		return result.comment_id;
	}

	/** Tells Orbit that a cancelled task run is stopped (or was not running here). */
	async taskRunStopped(runId: string): Promise<void> {
		await this.internal("POST", `/task-runs/${runId}/stopped`);
	}

	/** Records the end of a routine run; a second report of the same run changes nothing. */
	async finishRoutineRun(runId: string, report: RunReport): Promise<void> {
		await this.internal("PUT", `/routine-runs/${runId}`, report);
	}

	async #session(userId: string, workspaceId: string, agentUserId: string | null, threadKey: string | null, fresh: boolean): Promise<string> {
		// Per thread: a test run's thread gets a session whose writes Orbit refuses.
		const key = `${userId}:${workspaceId}:${agentUserId ?? ""}:${threadKey ?? ""}`;
		const cached = this.#sessions.get(key);
		if (!fresh && cached && cached.expiresAt - Date.now() > 60_000) return cached.token;
		const issued = (await this.internal("POST", "/sessions", {
			user_id: userId,
			workspace_id: workspaceId,
			...(agentUserId ? { agent_user_id: agentUserId } : {}),
			...(threadKey ? { thread_key: threadKey } : {}),
		})) as {
			token: string;
			expires_at: number;
		};
		this.#sessions.set(key, { token: issued.token, expiresAt: issued.expires_at });
		return issued.token;
	}

	/**
	 * Orbit's public API as `userId`, below `/api/v1/workspaces/{workspaceId}`. Everything the person cannot do
	 * fails here, exactly as in their browser. `agentUserId`: a custom agent's session, which Orbit limits to the
	 * routes of its tools. `threadKey`: the run's thread, which decides whether Orbit refuses its writes (a test run).
	 */
	async asUser(userId: string, workspaceId: string, method: string, path: string, body?: unknown, agentUserId: string | null = null, threadKey: string | null = null): Promise<unknown> {
		const call = async (fresh: boolean) =>
			this.#fetch(
				method,
				`/api/v1/workspaces/${workspaceId}${path}`,
				{ authorization: `Bearer ${await this.#session(userId, workspaceId, agentUserId, threadKey, fresh)}` },
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
