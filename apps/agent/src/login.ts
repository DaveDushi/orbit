/**
 * ChatGPT sign-in for one person with the device-code flow: Orbit shows a code, the person enters it at
 * auth.openai.com on any device, and this process polls OpenAI until it is done. No callback port is opened, so it
 * works on a remote server and for many people at once.
 */

import type { MutableModels } from "@earendil-works/pi-ai";

import { PROVIDER } from "./models.ts";
import type { OrbitClient } from "./orbit.ts";

export type LoginView = {
	state: "pending" | "connected" | "failed";
	user_code?: string;
	verification_uri?: string;
	expires_at?: number;
	error?: string;
};

type Login = { view: LoginView; abort: AbortController };

export class Logins {
	readonly #logins = new Map<string, Login>();
	readonly #models: (userId: string) => MutableModels;
	readonly #orbit: OrbitClient;
	readonly #mock: boolean;

	constructor(models: (userId: string) => MutableModels, orbit: OrbitClient, mock: boolean) {
		this.#models = models;
		this.#orbit = orbit;
		this.#mock = mock;
	}

	view(userId: string): LoginView | null {
		const login = this.#logins.get(userId);
		if (!login) return null;
		if (login.view.state === "pending" && login.view.expires_at && login.view.expires_at < Date.now()) {
			login.abort.abort();
			login.view = { state: "failed", error: "The code expired. Start again." };
		}
		return login.view;
	}

	cancel(userId: string): void {
		this.#logins.get(userId)?.abort.abort();
		this.#logins.delete(userId);
	}

	/** Starts a sign-in, or returns the one that is pending. Resolves once the code is known (or it failed). */
	async start(userId: string): Promise<LoginView> {
		const current = this.view(userId);
		if (current?.state === "pending") return current;
		this.cancel(userId);
		const login: Login = { view: { state: "pending" }, abort: new AbortController() };
		this.#logins.set(userId, login);
		let shown!: () => void;
		const codeShown = new Promise<void>((resolve) => (shown = resolve));
		const done = this.#mock ? this.#mockLogin(userId, login, shown) : this.#deviceLogin(userId, login, shown);
		done
			.then(() => {
				login.view = { state: "connected" };
			})
			.catch((error: unknown) => {
				if (this.#logins.get(userId) !== login) return;
				login.view = { state: "failed", error: error instanceof Error ? error.message : String(error) };
			})
			.finally(shown);
		await Promise.race([codeShown, new Promise((resolve) => setTimeout(resolve, 15_000))]);
		return login.view;
	}

	async #deviceLogin(userId: string, login: Login, shown: () => void): Promise<void> {
		await this.#models(userId).login(PROVIDER, "oauth", {
			signal: login.abort.signal,
			prompt: async (prompt) => {
				if (prompt.type === "select" && prompt.options.some((option) => option.id === "device_code")) return "device_code";
				throw new Error("This sign-in needs a step that Orbit does not support.");
			},
			notify: (event) => {
				if (event.type !== "device_code") return;
				login.view = {
					state: "pending",
					user_code: event.userCode,
					verification_uri: event.verificationUri,
					expires_at: Date.now() + (event.expiresInSeconds ?? 900) * 1000,
				};
				shown();
			},
		});
	}

	/** AGENT_MOCK: a fake code that "completes" after three seconds with a fake credential. */
	async #mockLogin(userId: string, login: Login, shown: () => void): Promise<void> {
		login.view = {
			state: "pending",
			user_code: `MOCK-${Math.random().toString(36).slice(2, 6).toUpperCase()}`,
			verification_uri: "https://auth.openai.com/codex/device",
			expires_at: Date.now() + 900_000,
		};
		shown();
		await new Promise((resolve) => setTimeout(resolve, 3_000));
		if (login.abort.signal.aborted) throw new Error("Login cancelled");
		await this.#orbit.writeCredential(userId, {
			type: "oauth",
			access: "mock-access-token",
			refresh: "mock-refresh-token",
			expires: Date.now() + 10 * 365 * 86_400_000,
			accountId: "mock-chatgpt-account",
		});
	}
}
