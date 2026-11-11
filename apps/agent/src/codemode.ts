/**
 * Code mode for every agent (`@earendil-works/pi-durable/codemode`): the model writes JavaScript that calls the agent's
 * own tools as `await tools.<name>(args)` in a QuickJS sandbox (`@earendil-works/pi-codemode`), which has no network,
 * no files, no timers and no modules. Each call is a nested tool call: it goes through the same hooks as a call of the
 * model (a custom agent's tool list, the run's tool-call limit) and the same Orbit checks (the driver's permissions,
 * the agent's connected services, the driver's yes to a write). Scripts never reach `codemode` or `run_saved_script`
 * themselves (`callers: ["model"]`), nor `ask_teammate`.
 *
 * `run_saved_script` runs a routine's saved script: the exact code Orbit stored with the run (`ThreadDoc.routineRun`),
 * never code from the model, so the creator's yes to that code (its hash) is what Orbit checks. In such a run it is the
 * only tool offered to the model (`Agent.#startRun`). A script's `store()`/`load()` values are kept per routine: the
 * run starts with the routine's saved values and reports the new ones at the end.
 */

import type { Context } from "@earendil-works/chord/context";
import { Type } from "@earendil-works/pi-ai";
import { CodemodeSandbox } from "@earendil-works/pi-codemode";
import { defineTool } from "@earendil-works/pi-durable";
import type { ToolExecutionApi } from "@earendil-works/pi-durable";
import { createCodemodeExtension, createCodemodeTool, type CodemodeToolOptions } from "@earendil-works/pi-durable/codemode";

import { ThreadDoc } from "./docs.ts";

export const CODEMODE_TOOL = "codemode";
export const RUN_SCRIPT_TOOL = "run_saved_script";

/** A script's heap; allocations beyond it fail inside the script. */
const MEMORY_LIMIT_BYTES = 64 * 1024 * 1024;
/** A script's deadline, its tool calls included. */
const TIMEOUT_MS = 5 * 60 * 1000;

let sandbox: CodemodeSandbox | undefined;

/** One sandbox (a worker) for the process, made on first use. */
function options(): CodemodeToolOptions {
	sandbox ??= new CodemodeSandbox({ memoryLimitBytes: MEMORY_LIMIT_BYTES, timeoutMs: TIMEOUT_MS });
	// No `models.*`: a script makes no model calls of its own.
	return { sandbox, models: false };
}

/** Lazy, so importing the tools (tests, the catalog) starts no worker: only a script run reads the sandbox. */
const lazyOptions = new Proxy({} as CodemodeToolOptions, { get: (_target, key) => (key === "models" ? false : options()[key as keyof CodemodeToolOptions]) });

const codemode = createCodemodeTool(lazyOptions);

const runSavedScript = defineTool({
	...codemode,
	name: RUN_SCRIPT_TOOL,
	description:
		"Run this routine's saved script (the exact code its creator saved; you cannot change it), then answer with its result. It calls the agent's tools; in a test run, writes are not done and come back as \"would do\".",
	parameters: Type.Object({}),
	constrainedSampling: undefined,
	execute: async (_args: Record<string, never>, api: ToolExecutionApi, context: Context) => {
		const state = await api.snapshot(ThreadDoc, api.conversationId, context);
		const script = state?.routineRun?.script;
		if (!script) return { output: [{ type: "text", text: "This run has no saved script." }], isError: true };
		return codemode.execute({ code: script.code }, api, context);
	},
});

/** `run_saved_script`: the only tool a script run offers the model, and one no other run offers. */
export const RUN_SCRIPT = runSavedScript;

/** The code mode tools: `codemode` for the model, `run_saved_script` for a routine's script runs. */
export const CODE_TOOLS = [codemode, runSavedScript];

/** The `codemode` prompt section: the tools scripts can call, as TypeScript declarations. */
export const CODEMODE_SECTIONS = createCodemodeExtension(lazyOptions).sections ?? [];

/** Closes the sandbox's worker, when one was made. */
export async function closeCodemode(): Promise<void> {
	await sandbox?.close();
	sandbox = undefined;
}
