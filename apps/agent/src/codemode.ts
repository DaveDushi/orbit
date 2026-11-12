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
 * only tool offered to the model (`Agent.#startRun`), with `repair_routine_script` in a scheduled run: when the script
 * fails, the model may save a fixed version through Orbit (at most `MAX_SCRIPT_REPAIRS` times), which then stores the
 * new code with the run and makes the rest of the run write nothing until the creator approves that version. A
 * script's `store()`/`load()` values are kept per routine: the run starts with the routine's saved values and reports
 * the new ones at the end.
 */

import type { Context } from "@earendil-works/chord/context";
import { Type } from "@earendil-works/pi-ai";
import { CodemodeSandbox } from "@earendil-works/pi-codemode";
import { defineTool } from "@earendil-works/pi-durable";
import type { ToolExecutionApi } from "@earendil-works/pi-durable";
import { createCodemodeExtension, createCodemodeTool, type CodemodeToolOptions } from "@earendil-works/pi-durable/codemode";

import { ThreadDoc, type RoutineRunState } from "./docs.ts";

export const CODEMODE_TOOL = "codemode";
export const RUN_SCRIPT_TOOL = "run_saved_script";
export const REPAIR_SCRIPT_TOOL = "repair_routine_script";
/** Repairs of a routine's failed script in one run; then the run answers with the error. */
export const MAX_SCRIPT_REPAIRS = 2;

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
		"Run this routine's saved script (the exact code Orbit gave this run), then answer with its result. It calls the agent's tools; in a test run, writes are not done and come back as \"would do\". When it fails, the result says whether you may repair it.",
	parameters: Type.Object({}),
	constrainedSampling: undefined,
	execute: async (_args: Record<string, never>, api: ToolExecutionApi, context: Context) => {
		const state = await api.snapshot(ThreadDoc, api.conversationId, context);
		const script = state?.routineRun?.script;
		if (!script) return { output: [{ type: "text", text: "This run has no saved script." }], isError: true };
		const result = await codemode.execute({ code: script.code }, api, context);
		if (!result.isError || !state?.routineRun) return result;
		return { ...result, output: [...(result.output ?? []), { type: "text", text: repairHint(state.routineRun) }] };
	},
});

/** What the model may do after the routine's script failed: repair it (not in a test run, at most twice), or report. */
export function repairHint(run: Pick<RoutineRunState, "test" | "repairs">): string {
	if (run.test) return "The saved script failed. This is a test run, so you cannot change the script: answer with the error and say how the script could be fixed.";
	const done = run.repairs?.length ?? 0;
	if (done >= MAX_SCRIPT_REPAIRS) return `The saved script failed again, and this run has no repairs left (${MAX_SCRIPT_REPAIRS} per run). Answer with the error and stop.`;
	return `The saved script failed. If you can see the cause, fix it: call ${REPAIR_SCRIPT_TOOL} with the whole corrected script and one line on what you changed, then call ${RUN_SCRIPT_TOOL} again (repair ${done + 1} of ${MAX_SCRIPT_REPAIRS}). The fixed version makes no writes until the driver approves it; reads and your answer in this thread still work. If you cannot fix it, answer with the error.`;
}

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
