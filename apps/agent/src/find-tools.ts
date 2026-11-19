/**
 * Code mode is how every agent works: the model is offered only `find_tools` and `codemode` (`MODEL_TOOLS`, with the
 * few tools a script cannot stand in for), and reaches every other tool from a script as `await tools.<name>(args)`.
 * Each nested call goes through the same hooks and Orbit checks as a direct one: the agent's tool list, the driver's
 * permissions and yes, the run's tool-call limit.
 *
 * `find_tools` searches the run's callable Orbit tools and the driver's tools of connected services (remote MCP servers,
 * HTTP APIs), which scripts call through `use_integration`. Those lists can have hundreds of tools, so neither is in
 * the prompt: the run keeps the services' tools in `IntegrationToolsDoc` (read at its start, `Agent.#startRun`), and
 * `find_tools` returns only the best matches, with their arguments.
 */

import type { Context } from "@earendil-works/chord/context";
import { Type } from "@earendil-works/pi-ai";
import { defineDoc, defineTool, type ToolExecutionApi } from "@earendil-works/pi-durable";

import type { CodemodeSandboxBase } from "@earendil-works/pi-codemode";

import { CODEMODE_TOOL, readOnlySandbox, REPAIR_SCRIPT_TOOL, RUN_SCRIPT_TOOL } from "./codemode.ts";
import { ThreadDoc } from "./docs.ts";
import { untrusted } from "./files.ts";
import type { IntegrationTool } from "./integrations.ts";

export const FIND_TOOLS_TOOL = "find_tools";
export const INTEGRATION_TOOL = "use_integration";

/**
 * The only tools offered to the model, when the run has them (an allow list: a tool not named here, such as a new Orbit
 * tool, `sandbox_shell` or any tool of a connected service, is reached only from a script). The two main ones; the jobs
 * (`plan_job` stores a script for one approval, `run_job` runs it for up to 60 minutes, longer than a script may run:
 * ./jobs.ts); and those a script cannot stand in for: `view_image` (a script gets only the text of a result, so images
 * would never reach the model), `ask_teammate` (it ends the run with a question), and the skill tools.
 */
export const MODEL_TOOLS = [FIND_TOOLS_TOOL, CODEMODE_TOOL, "plan_job", "run_job", "view_image", "ask_teammate", "load_skill", "read_skill_file"];

/** The tools a run offers the model among its enabled `tools`, in their order. */
export function offeredTools<T extends { name: string }>(tools: readonly T[]): T[] {
	return tools.filter((tool) => MODEL_TOOLS.includes(tool.name));
}

/** The driver's tools of connected services in the current run, read at its start; empty without `use_integration`. */
export const IntegrationToolsDoc = defineDoc<{ tools: IntegrationTool[] }>({
	kind: "orbit.integration-tools",
	version: 1,
	scope: "conversation",
	history: "latest",
	fork: "current",
	initial: () => ({ tools: [] }),
	checkpointWhen: () => true,
});

const LIMIT = 8;
/** Characters of one tool's arguments schema in a result. */
const SCHEMA_CHARS = 2_000;

const STOP_WORDS = new Set(["a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "is", "it", "of", "on", "or", "that", "the", "this", "to", "with", "my", "me", "i"]);

/** `issues` → `issue`, `searches` → `search`. */
function stem(term: string): string {
	if (term.length > 4 && term.endsWith("ies")) return `${term.slice(0, -3)}y`;
	if (term.length > 4 && /(ches|shes|sses|xes|zes)$/.test(term)) return term.slice(0, -2);
	if (term.length > 2 && term.endsWith("s") && !term.endsWith("ss")) return term.slice(0, -1);
	return term;
}

/** Lowercase search terms, split at non-alphanumerics, without stop words; a camelCase word also gives its parts ("TikTok": tiktok, tik, tok). */
export function terms(text: string): string[] {
	return text
		.split(/[^A-Za-z0-9]+/)
		.flatMap((word) => {
			const parts = word.replace(/([a-z0-9])([A-Z])/g, "$1 $2").split(" ");
			return parts.length > 1 ? [word, ...parts] : [word];
		})
		.map((term) => term.toLowerCase())
		.filter((term) => term && !STOP_WORDS.has(term))
		.map(stem);
}

/** The property names of a JSON Schema object, also nested ones; anything else has none. */
export function schemaWords(schema: unknown, depth = 0): string[] {
	if (!schema || typeof schema !== "object" || depth > 3) return [];
	const properties = (schema as { properties?: unknown }).properties;
	if (!properties || typeof properties !== "object" || Array.isArray(properties)) return [];
	return Object.entries(properties as Record<string, unknown>).flatMap(([name, value]) => [name, ...schemaWords(value, depth + 1)]);
}

/** A searchable tool: what it is called as, the words of its name, and the rest of its text. */
export type Entry = { key: string; name: string; text: string };

/** Okapi BM25 (k1 1.2, b 0.75) of each entry's `field` for the `wanted` terms. */
function bm25(wanted: readonly string[], fields: readonly string[]): number[] {
	const counts = fields.map((field) => {
		const count = new Map<string, number>();
		for (const term of terms(field)) count.set(term, (count.get(term) ?? 0) + 1);
		return count;
	});
	const lengths = counts.map((count) => [...count.values()].reduce((sum, n) => sum + n, 0));
	const average = lengths.reduce((sum, n) => sum + n, 0) / fields.length || 1;
	return counts.map((count, index) =>
		wanted.reduce((score, term) => {
			const n = count.get(term) ?? 0;
			if (!n) return score;
			const have = counts.filter((other) => other.has(term)).length;
			const idf = Math.log(1 + (fields.length - have + 0.5) / (have + 0.5));
			return score + (idf * n * 2.2) / (n + 1.2 * (0.25 + (0.75 * lengths[index]) / average));
		}, 0),
	);
}

/**
 * The keys of the best matches for `query`, best first; ties keep the entries' order. The name and the text are scored
 * apart (BM25F-like), the name double, so a long description does not hide a tool whose name says what it does; the
 * sum is weighted by the share of the query's words the tool has.
 */
export function rank(query: string, entries: readonly Entry[], limit = LIMIT): string[] {
	const wanted = [...new Set(terms(query))];
	if (!wanted.length || !entries.length) return [];
	const names = bm25(wanted, entries.map((entry) => entry.name));
	const texts = bm25(wanted, entries.map((entry) => entry.text));
	// The share of the query's words a tool has: a tool that matches every word beats one that repeats a common one.
	const covered = entries.map((entry) => {
		const all = new Set([...terms(entry.name), ...terms(entry.text)]);
		return wanted.filter((term) => all.has(term)).length / wanted.length;
	});
	return entries
		.map((entry, index) => ({ key: entry.key, score: (2 * names[index] + texts[index]) * covered[index], index }))
		.filter((item) => item.score > 0)
		.sort((a, b) => b.score - a.score || a.index - b.index)
		.slice(0, limit)
		.map((item) => item.key);
}

type OrbitTool = { name: string; description: string; parameters: unknown };

/** The search entry of an Orbit tool, and of a tool of a connected service (also by its service's name). */
export const orbitEntry = (tool: OrbitTool): Entry => ({ key: tool.name, name: tool.name, text: `${tool.description} ${schemaWords(tool.parameters).join(" ")}` });
export const integrationEntry = (tool: IntegrationTool): Entry => ({
	key: `mcp:${tool.name}`,
	name: tool.name.split("__").slice(1).join(" "),
	text: `${tool.name.split("__")[0]} ${tool.integration} ${tool.description} ${schemaWords(tool.input_schema).join(" ")}`,
});

const schemaText = (schema: unknown) => {
	const text = JSON.stringify(schema ?? {}) ?? "{}";
	return text.length > SCHEMA_CHARS ? `${text.slice(0, SCHEMA_CHARS)}…` : text;
};

const APPROVAL = { never: "", always: " Needs the driver's approval.", writes: " Methods other than GET and HEAD need the driver's approval." };

/** The result for the matches: Orbit tools first as they rank, then the services' tools, fenced as data from outside. */
export function foundText(query: string, found: readonly string[], orbit: readonly OrbitTool[], services: readonly IntegrationTool[]): string {
	if (!found.length) {
		const names = services.length ? `; tools of connected services: ${[...new Set(services.map((tool) => tool.integration))].join(", ")}` : "";
		return `No tool matches "${query}". Try other words. Your tools: ${orbit.map((tool) => tool.name).join(", ")}${names}.`;
	}
	const own: string[] = [];
	const outside: string[] = [];
	for (const key of found) {
		if (key.startsWith("mcp:")) {
			const tool = services.find((candidate) => `mcp:${candidate.name}` === key)!;
			outside.push(
				`${tool.name} (${tool.integration}, outside Orbit):${APPROVAL[tool.approval] ?? ""} ${tool.description.replace(/\s+/g, " ").slice(0, 600)}\ncall: await tools.use_integration({ tool: "${tool.name}", arguments: {…} })\narguments: ${schemaText(tool.input_schema)}`,
			);
			continue;
		}
		const tool = orbit.find((candidate) => candidate.name === key)!;
		own.push(`${tool.name}: ${tool.description}\ncall: await tools.${tool.name}({…})\narguments: ${schemaText(tool.parameters)}`);
	}
	return [...own, ...(outside.length ? [untrusted("tools of connected services", outside.join("\n\n"))] : [])].join("\n\n");
}

/** One tool as a `find_tools` script sees it in `catalog`. `approval`: `null` for Orbit tools (Orbit decides per call). */
export type CatalogEntry = { name: string; description: string; input_schema: unknown; integration: string | null; approval: IntegrationTool["approval"] | null; call: string };

export function catalogOf(orbit: readonly OrbitTool[], services: readonly IntegrationTool[]): CatalogEntry[] {
	return [
		...orbit.map((tool) => ({ name: tool.name, description: tool.description, input_schema: tool.parameters ?? {}, integration: null, approval: null, call: `await tools.${tool.name}({…})` })),
		...services.map((tool) => ({
			name: tool.name,
			description: tool.description,
			input_schema: tool.input_schema ?? {},
			integration: tool.integration,
			approval: tool.approval,
			call: `await tools.use_integration({ tool: "${tool.name}", arguments: {…} })`,
		})),
	];
}

/**
 * Runs a model's read-only script over `entries` (the global `catalog`) in a sandbox without tools or globals: it can
 * filter and return, nothing else (no tool calls, no network, no files, no writes; `store()` is dropped). The result is
 * fenced as data: it can hold the services' text.
 */
export async function catalogScript(code: string, entries: readonly CatalogEntry[], sandbox: CodemodeSandboxBase = readOnlySandbox()): Promise<{ text: string; isError: boolean }> {
	// The catalog is JSON, so it is a valid JavaScript literal; the script runs right after it.
	const result = await sandbox.execute(`const catalog = ${JSON.stringify(entries)};\n${code}`);
	if (!result.ok) return { text: untrusted("find_tools script error", `${result.error.kind}: ${result.error.message}`.slice(0, 2_000)), isError: true };
	const printed = result.output.flatMap((item) => (item.type === "text" ? [item.text] : [])).join("\n");
	const value = result.value === undefined ? printed : typeof result.value === "string" ? result.value : (JSON.stringify(result.value, null, 1) ?? "");
	return { text: untrusted("tool catalog", value || "(the script returned nothing)"), isError: false };
}

export const FIND_TOOLS = defineTool({
	name: FIND_TOOLS_TOOL,
	callers: ["model"],
	description: `Find the tools you call from codemode. Either by words (query), such as "drive trash", "routine" or "tiktok campaign budget": at most ${LIMIT} tools with their names, descriptions, how to call them and their arguments (JSON Schema). Or with a read-only JavaScript script (code) over the global \`catalog\`, an array of { name, description, input_schema, integration (null: an Orbit tool), approval ("never", "always", "writes"; null: Orbit decides), call }; return what you need, for example \`return catalog.filter((t) => t.integration === "TikTok Ads" && t.approval === "never").map((t) => t.name)\`. A script here cannot call tools. Searches your Orbit tools and the tools of connected services.`,
	parameters: Type.Object({
		query: Type.Optional(Type.String({ minLength: 1, maxLength: 200, description: "Words for what you want to do" })),
		code: Type.Optional(Type.String({ minLength: 1, maxLength: 10_000, description: "Read-only JavaScript over `catalog`; return the matches" })),
	}),
	execute: async (args: { query?: string; code?: string }, api: ToolExecutionApi, context: Context) => {
		if (Boolean(args.query) === Boolean(args.code)) return { output: [{ type: "text", text: "Give either query (words) or code (a script over catalog)." }], isError: true };
		const state = await api.snapshot(ThreadDoc, api.conversationId, context);
		const allowed = (name: string) => !state?.allowedTools || state.allowedTools.includes(name);
		// The run's callable tools are the agent's own (`configure`); its list is checked again here.
		const all = (await api.agent(context)).callable;
		const callable = all.filter((tool) => !MODEL_TOOLS.includes(tool.name) && ![RUN_SCRIPT_TOOL, REPAIR_SCRIPT_TOOL, INTEGRATION_TOOL].includes(tool.name) && allowed(tool.name));
		const integrations = all.some((tool) => tool.name === INTEGRATION_TOOL) && allowed(INTEGRATION_TOOL);
		const services = integrations ? ((await api.snapshot(IntegrationToolsDoc, api.conversationId, context))?.tools ?? []) : [];
		if (args.code) {
			const { text, isError } = await catalogScript(args.code, catalogOf(callable, services));
			return { output: [{ type: "text", text }], ...(isError ? { isError } : {}) };
		}
		const found = rank(args.query!, [...callable.map(orbitEntry), ...services.map(integrationEntry)]);
		return { output: [{ type: "text", text: foundText(args.query!, found, callable, services) }] };
	},
});
