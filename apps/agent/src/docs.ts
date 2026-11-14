import { defineDoc } from "@earendil-works/pi-durable";

/** The run that answers one input: its submission and the progress message it edits. */
export type RunState = {
	inputId: string;
	/** A Pi `SubmissionId`. */
	submissionId: number | null;
	progressMessageId: string | null;
	startedAt: number;
	/** A line above the progress and the answer, such as a routine's name; absent: none. */
	title?: string | null;
};

/** A routine run (Orbit's `agent_routine_runs`): what it may write, and the saved script it runs. */
export type RoutineRunState = {
	runId: string;
	routineName: string;
	/** A test run: in the creator's DM, nothing written (Orbit refuses the writes). */
	test: boolean;
	/** `normal`, `script` or `dry`, as Orbit decided (and enforces). */
	writes: string;
	/** The exact code Orbit stored with the run; `run_saved_script` runs only this. */
	script: { name: string; code: string; hash: string } | null;
	/** The end of the run was reported to Orbit. */
	reported: boolean;
	/** The script's repairs in this run (`repair_routine_script`), oldest first; at most `MAX_SCRIPT_REPAIRS`. */
	repairs?: ScriptRepair[];
};

/** One repair of a routine's script in a run: the version that failed, the new one, and what changed. */
export type ScriptRepair = { from: string; to: string; change: string; added: number; removed: number };

/**
 * The task run of a thread (Orbit's `agent_task_runs`): the run whose input is `runId` writes its progress and result
 * into one comment on the task; a later run in the same thread (after an answer or a yes) writes its result there too.
 */
export type TaskRunState = {
	runId: string;
	taskId: string;
	identifier: string;
	title: string;
	/** Set when Orbit cancelled the run: why (the agent was unassigned, the task deleted). */
	stopped?: string | null;
};

/** One tool call of a routine run, for its history. `would_do`: a write a dry run did not make. */
export type RunCall = { name: string; args: string; status: "ok" | "error" | "dry_run"; would_do?: string };

/**
 * One Orbit chat thread's agent state, kept in its Pi conversation. Tools read the driver from here, so a
 * tool always acts as the person whose run it is in.
 */
export type ThreadState = {
	threadKey: string;
	workspaceId: string;
	conversationId: string;
	conversationName: string;
	conversationKind: string;
	/** People in the conversation, the agent included. Absent in threads made before memory existed. */
	conversationMembers?: number;
	/** `null`: the agent answers in the main list of a DM with it. */
	rootMessageId: string | null;
	driverId: string | null;
	driverName: string | null;
	run: RunState | null;
	/** Set by `ask_teammate`: the run ends and the thread waits for an answer. */
	waitingFor: string | null;
	toolCalls: number;
	/** The agent's name and the workspace's team instructions, read at the start of the current run. */
	agentName?: string;
	teamInstructions?: string;
	/** The thread's custom agent (its bot user); absent or `null`: the default agent. Fixed for the thread. */
	agentUserId?: string | null;
	/** The agent's instructions (a custom agent's, or the default agent's card) and a custom agent's tools, read at the start of the current run; `null` tools: every tool. */
	agentInstructions?: string;
	allowedTools?: string[] | null;
	/** The default agent's name, so a custom agent can point people to it. */
	defaultAgentName?: string;
	/** The memory section of the current run (the wake view read at its start), as the model sees it. */
	memory?: string | null;
	/** The integration tools section of the current run (the driver's tools, read at its start). */
	integrations?: string | null;
	/** The skills section of the current run (names and descriptions of the agent's skills, read at its start). */
	skills?: string | null;
	/** Summaries Orbit asked this thread's runs to write: `scope:lo-hi` → claim. */
	claims?: Record<string, string>;
	/** The routine run the current run is; `null` or absent: a chat run. */
	routineRun?: RoutineRunState | null;
	/** The tool calls of the current routine run (scripts' calls included), oldest first. */
	runCalls?: RunCall[];
	/** The task run that made this thread; `null` or absent: a chat thread. */
	taskRun?: TaskRunState | null;
};

export const ThreadDoc = defineDoc<ThreadState>({
	kind: "orbit.thread",
	version: 1,
	scope: "conversation",
	history: "latest",
	fork: "current",
	initial: () => ({
		threadKey: "",
		workspaceId: "",
		conversationId: "",
		conversationName: "",
		conversationKind: "",
		rootMessageId: null,
		driverId: null,
		driverName: null,
		run: null,
		waitingFor: null,
		toolCalls: 0,
	}),
	checkpointWhen: () => true,
});

/** On the root conversation: which Pi conversation serves which Orbit thread. */
export type IndexState = { threads: Record<string, number> };

export const IndexDoc = defineDoc<IndexState>({
	kind: "orbit.index",
	version: 1,
	scope: "conversation",
	history: "latest",
	fork: "initial",
	initial: () => ({ threads: {} }),
	checkpointWhen: () => true,
});
