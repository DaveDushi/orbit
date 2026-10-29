import { readFileSync } from "node:fs";
import { join } from "node:path";

/** The sidecar's settings, from the environment. See docs/agent.md. */
export type Config = {
	/** Orbit's base URL as the sidecar reaches it, e.g. `http://127.0.0.1:8000`. */
	readonly orbitUrl: string;
	/** Orbit's public URL, for links in messages, e.g. `https://orbit.example.com`. */
	readonly publicUrl: string;
	readonly secret: string;
	readonly host: string;
	readonly port: number;
	readonly dataDir: string;
	/** The ChatGPT (Codex) model id. */
	readonly model: string;
	/** A scripted model and a fake ChatGPT sign-in, for tests without OpenAI. */
	readonly mock: boolean;
	/** Sent to Orbit, which trusts it only from a configured proxy address (loopback in production). */
	readonly forwardedProto: string | undefined;
	readonly maxRunsPerUser: number;
	readonly maxToolCallsPerRun: number;
	/** Tokens of memory shown at the start of each run (team and private together); 0 turns it off. */
	readonly memoryTokens: number;
	/** Tokens of context at which a conversation compacts in the background; 0 keeps Pi's defaults. */
	readonly compactTokens: number;
};

export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
	const secret = (env.ORBIT_AGENT_SECRET_FILE ? readFileSync(env.ORBIT_AGENT_SECRET_FILE, "utf8") : (env.ORBIT_AGENT_SECRET ?? "")).trim();
	if (secret.length < 32) throw new Error("ORBIT_AGENT_SECRET must have at least 32 characters");
	const orbitUrl = (env.ORBIT_URL ?? "").replace(/\/+$/, "");
	if (!orbitUrl) throw new Error("Set ORBIT_URL to Orbit's address, for example http://127.0.0.1:8000");
	const compactTokens = Number(env.AGENT_COMPACT_TOKENS || 250_000);
	if (!Number.isSafeInteger(compactTokens) || compactTokens < 0) throw new Error("AGENT_COMPACT_TOKENS must be a whole number of tokens, or 0 for Pi's defaults");
	return {
		orbitUrl,
		publicUrl: (env.ORBIT_PUBLIC_URL ?? orbitUrl).replace(/\/+$/, ""),
		secret,
		host: env.AGENT_HOST ?? "127.0.0.1",
		port: Number(env.AGENT_PORT ?? 8790),
		dataDir: env.AGENT_DATA_DIR ?? join(process.cwd(), "data"),
		model: env.AGENT_MODEL ?? "gpt-6.1-sol",
		mock: env.AGENT_MOCK === "1",
		forwardedProto: env.ORBIT_FORWARDED_PROTO || undefined,
		maxRunsPerUser: Number(env.AGENT_MAX_RUNS_PER_USER ?? 3),
		maxToolCallsPerRun: Number(env.AGENT_MAX_TOOL_CALLS ?? 40),
		memoryTokens: Number(env.AGENT_MEMORY_TOKENS ?? 3_000),
		compactTokens,
	};
}
