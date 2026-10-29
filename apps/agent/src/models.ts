/**
 * Model access per person. Every person's ChatGPT credential stays in Orbit (sealed); each person gets a pi-ai
 * `Models` whose credential store reads and writes it there, so token refreshes are saved for them alone.
 *
 * Pi's Harness takes one `Models`. `routedModels` sends each request to the `Models` of the person who pays for the
 * conversation's current run, found by the conversation's provider session id (Pi passes it as `sessionId`).
 */

import { createModels, type Credential, type CredentialInfo, type CredentialStore, type Models, type MutableModels } from "@earendil-works/pi-ai";
import { openaiCodexProvider } from "@earendil-works/pi-ai/providers/openai-codex";

import type { OrbitClient } from "./orbit.ts";

/** pi-ai's ChatGPT (Codex) provider; its OAuth flow has a device-code login. */
export const PROVIDER = "openai-codex";

/** A person's credential, stored sealed in Orbit. `modify` calls are serialized per person. */
export class OrbitCredentialStore implements CredentialStore {
	readonly #orbit: OrbitClient;
	readonly #userId: string;
	#chain: Promise<unknown> = Promise.resolve();

	constructor(orbit: OrbitClient, userId: string) {
		this.#orbit = orbit;
		this.#userId = userId;
	}

	async read(providerId: string): Promise<Credential | undefined> {
		if (providerId !== PROVIDER) return undefined;
		return (await this.#orbit.readCredential(this.#userId)) as Credential | undefined;
	}

	async list(): Promise<readonly CredentialInfo[]> {
		const credential = await this.read(PROVIDER);
		return credential ? [{ providerId: PROVIDER, type: credential.type }] : [];
	}

	modify(providerId: string, fn: (current: Credential | undefined) => Promise<Credential | undefined>): Promise<Credential | undefined> {
		const next = this.#chain.then(async () => {
			const current = await this.read(providerId);
			const changed = await fn(current);
			if (providerId !== PROVIDER) return changed;
			if (changed) await this.#orbit.writeCredential(this.#userId, changed as unknown as Record<string, unknown>);
			else if (current) await this.#orbit.deleteCredential(this.#userId);
			return changed;
		});
		this.#chain = next.catch(() => undefined);
		return next;
	}

	async delete(providerId: string): Promise<void> {
		if (providerId === PROVIDER) await this.#orbit.deleteCredential(this.#userId);
	}
}

/** One `Models` per person, made on first use. */
export class PeopleModels {
	readonly #orbit: OrbitClient;
	readonly #models = new Map<string, MutableModels>();

	constructor(orbit: OrbitClient) {
		this.#orbit = orbit;
	}

	of(userId: string): MutableModels {
		let models = this.#models.get(userId);
		if (!models) {
			models = createModels({ credentials: new OrbitCredentialStore(this.#orbit, userId) });
			models.setProvider(openaiCodexProvider());
			this.#models.set(userId, models);
		}
		return models;
	}
}

const ROUTED = new Set(["stream", "complete", "streamSimple", "completeSimple", "streamDeferred", "fetchDeferred", "cancelDeferred"]);

/**
 * A `Models` that answers catalog questions from `base` and sends requests to `forPayer(payer)`, where the payer is
 * looked up from the request's `sessionId`. A request without a known payer goes to `base`, which has no credential
 * and fails with an auth error.
 */
export function routedModels(base: Models, payerOf: (sessionId: string | undefined) => string | undefined, forPayer: (userId: string) => Models): Models {
	return new Proxy(base, {
		get(target, property, receiver) {
			if (typeof property === "string" && ROUTED.has(property)) {
				return (model: unknown, context: unknown, options?: { sessionId?: string }) => {
					const payer = payerOf(options?.sessionId);
					const models = payer ? forPayer(payer) : target;
					return (models as unknown as Record<string, (...args: unknown[]) => unknown>)[property](model, context, options);
				};
			}
			const value = Reflect.get(target, property, receiver);
			return typeof value === "function" ? value.bind(target) : value;
		},
	});
}
