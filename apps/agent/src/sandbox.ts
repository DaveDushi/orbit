/**
 * The client of the media sandbox broker (deploy/donna-sandbox/broker.py): one Unix socket connection per request.
 * The broker runs bash as a separate Linux user in bubblewrap, without network, with a read-only system and only
 * the conversation's /work writable (a disk image of fixed size). Every /work file goes in and out through a run,
 * so this process never opens a file there.
 */

import { createHash } from "node:crypto";
import { createConnection } from "node:net";
import type { Readable } from "node:stream";

export type RunOptions = {
	/** Become "$1", "$2", … of the script. */
	readonly args?: readonly string[];
	readonly timeoutSeconds?: number;
	/** Sent to the script's stdin: exactly `stdinSize` bytes. */
	readonly stdin?: Buffer | Readable;
	readonly stdinSize?: number;
	readonly stdoutMax?: number;
	readonly stderrMax?: number;
};

export type RunResult = {
	readonly exit: number | null;
	readonly signal: number | null;
	readonly timedOut: boolean;
	readonly stdout: Buffer;
	readonly stderr: Buffer;
	readonly stdoutTruncated: boolean;
	readonly stderrTruncated: boolean;
};

export class SandboxError extends Error {}

/** The /work of a Pi conversation: a fixed-length name the broker accepts, whatever the id looks like. */
export function workId(conversationId: string): string {
	return createHash("sha256").update(conversationId).digest("hex").slice(0, 32);
}

export class Sandbox {
	readonly socketPath: string;

	constructor(socketPath: string) {
		this.socketPath = socketPath;
	}

	/** Runs `script` with bash in the conversation's /work. */
	async run(work: string, script: string, options: RunOptions = {}): Promise<RunResult> {
		const stdinSize = options.stdin ? (options.stdinSize ?? (Buffer.isBuffer(options.stdin) ? options.stdin.length : 0)) : 0;
		const frames = await this.#request(
			{
				op: "run",
				work,
				script,
				args: options.args ?? [],
				timeout: options.timeoutSeconds ?? 120,
				stdin: stdinSize,
				stdout_max: options.stdoutMax ?? 64 * 1024,
				stderr_max: options.stderrMax ?? 64 * 1024,
			},
			options.stdin,
		);
		const stdout: Buffer[] = [];
		const stderr: Buffer[] = [];
		let end: Record<string, unknown> | undefined;
		for (const [kind, payload] of frames) {
			if (kind === "o") stdout.push(payload);
			else if (kind === "e") stderr.push(payload);
			else if (kind === "x") end = JSON.parse(payload.toString("utf8"));
		}
		if (!end) throw new SandboxError("The sandbox ended without an answer.");
		if (typeof end.error === "string") throw new SandboxError(`The sandbox refused: ${end.error}`);
		return {
			exit: typeof end.exit === "number" ? end.exit : null,
			signal: typeof end.signal === "number" ? end.signal : null,
			timedOut: end.timed_out === true,
			stdout: Buffer.concat(stdout),
			stderr: Buffer.concat(stderr),
			stdoutTruncated: end.stdout_truncated === true,
			stderrTruncated: end.stderr_truncated === true,
		};
	}

	/** Deletes the /work of every conversation unused for `idleSeconds`. */
	async gc(idleSeconds: number): Promise<number> {
		const frames = await this.#request({ op: "gc", idle_seconds: idleSeconds });
		const end = frames.find(([kind]) => kind === "x");
		return end ? Number(JSON.parse(end[1].toString("utf8")).removed ?? 0) : 0;
	}

	#request(request: Record<string, unknown>, stdin?: Buffer | Readable): Promise<[string, Buffer][]> {
		return new Promise((resolve, reject) => {
			const socket = createConnection({ path: this.socketPath });
			const frames: [string, Buffer][] = [];
			let buffered: Buffer = Buffer.alloc(0);
			socket.on("connect", () => {
				socket.write(`${JSON.stringify(request)}\n`);
				if (!stdin) return;
				if (Buffer.isBuffer(stdin)) socket.write(stdin);
				else stdin.pipe(socket, { end: false });
			});
			socket.on("data", (chunk: Buffer) => {
				buffered = buffered.length ? Buffer.concat([buffered, chunk]) : chunk;
				while (buffered.length >= 5) {
					const length = buffered.readUInt32BE(1);
					if (buffered.length < 5 + length) break;
					frames.push([String.fromCharCode(buffered[0]), buffered.subarray(5, 5 + length)]);
					buffered = buffered.subarray(5 + length);
				}
			});
			socket.on("error", (error) => {
				// A script that does not read all its stdin ends the connection while we still write: its answer counts.
				if (frames.some(([kind]) => kind === "x")) resolve(frames);
				else reject(new SandboxError(`The sandbox is not available (${error.message}).`));
			});
			socket.on("close", () => resolve(frames));
			if (stdin && !Buffer.isBuffer(stdin)) stdin.on("error", (error) => socket.destroy(error));
		});
	}
}
