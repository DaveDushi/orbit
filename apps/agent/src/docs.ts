import { defineDoc } from "@earendil-works/pi-durable";

/** The run that answers one input: its submission and the progress message it edits. */
export type RunState = {
	inputId: string;
	/** A Pi `SubmissionId`. */
	submissionId: number | null;
	progressMessageId: string | null;
	startedAt: number;
};

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
