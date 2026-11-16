/**
 * Runs every case (./cases.ts) against a fake Orbit with Donna's tool setup (every tool offered directly, `codemode`
 * as one more tool) and the scripted model (./scripted-model.ts), and writes results-<label>.json. Usage: ./run.sh [label].
 * Copied from the donna-codemode eval (branch donna-codemode-eval), direct mode only.
 */

import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { Agent } from "../../apps/agent/src/agent.ts";
import type { Config } from "../../apps/agent/src/config.ts";
import { OrbitClient } from "../../apps/agent/src/orbit.ts";
import { CASES, type Case } from "./cases.ts";
import { ANN, BOT, fakeBroker, FakeOrbit, id, SECRET } from "./fake-orbit.ts";
import { scriptedModels, type RequestStat } from "./scripted-model.ts";

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const sum = (values: number[]) => values.reduce((total, value) => total + value, 0);

function evalConfig(orbit: FakeOrbit, dataDir: string, sandboxSocket: string): Config {
	return {
		orbitUrl: orbit.url,
		publicUrl: "https://orbit.test",
		secret: SECRET,
		host: "127.0.0.1",
		port: 0,
		dataDir,
		model: "gpt-test",
		mock: false,
		forwardedProto: undefined,
		maxRunsPerUser: 3,
		maxToolCallsPerRun: 40,
		memoryTokens: 3_000,
		compactTokens: 250_000,
		compactModel: undefined,
		compactThinking: "high",
		sandboxSocket,
		sandboxIdleHours: 24,
	};
}

async function until(what: string, check: () => boolean, ms = 30_000): Promise<void> {
	const end = Date.now() + ms;
	while (!check()) {
		if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
		await sleep(5);
	}
}

type CaseResult = { id: string; category: string; success: boolean; failure?: string; requests: number; inputTokens: number; outputTokens: number; fixedTokens: number; codemodeTokens: number; turns: string[]; answer: string };

async function runCase(orbit: FakeOrbit, stats: RequestStat[], item: Case): Promise<CaseResult> {
	const thread = id();
	const firstCall = orbit.calls.length;
	const firstStat = stats.length;
	const approvalsBefore = new Set([...orbit.messages.keys()].filter((key) => key.startsWith("approve:")));
	const done = (messageId: string) => () => {
		const status = orbit.threads.get(thread)?.status;
		const body = orbit.messages.get(`agent:${messageId}`)?.body ?? "";
		return (status === "idle" || status === "waiting") && body !== "" && !body.startsWith("⏳");
	};
	const first = orbit.add({ thread_key: thread, author_id: ANN, author_name: "Ann", kind: "instruction", body: `<@${BOT}> ${item.text} [eval:${item.id}]`, files: item.files ? [...item.files] : undefined });
	let failure: string | undefined;
	let last = first;
	try {
		await until(`${item.id} run`, done(first));
		if (item.ends === "approval") {
			if (orbit.threads.get(thread)?.status !== "waiting") throw new Error("no approval question");
			orbit.approve();
			last = orbit.add({ thread_key: thread, author_id: ANN, author_name: "Ann", kind: "answer", body: "yes" });
			await until(`${item.id} after yes`, done(last));
		}
	} catch (error) {
		failure = String(error);
	}
	const calls = orbit.calls.slice(firstCall).filter((call) => call.as === ANN);
	const mine = stats.slice(firstStat).filter((stat) => stat.caseId === item.id);
	const approvalPrompts = [...orbit.messages.entries()].filter(([key]) => key.startsWith("approve:") && !approvalsBefore.has(key)).map(([, message]) => message.body);
	const answer = [first, ...(last === first ? [] : [last])].map((messageId) => orbit.messages.get(`agent:${messageId}`)?.body ?? "").join("\n---\n");
	if (!failure) {
		const remaining = [...calls];
		for (const [method, path] of item.expect) {
			const index = remaining.findIndex((call) => call.method === method && path.test(call.path));
			if (index < 0) {
				failure = `missing ${method} ${path}`;
				break;
			}
			remaining.splice(index, 1);
		}
	}
	if (!failure && /a tool failed|I could not finish|no case/.test(answer)) failure = `answer: ${answer.slice(0, 300)}`;
	if (!failure && item.ends === "approval" && (approvalPrompts.length !== 1 || !orbit.done.length)) failure = `approval: ${approvalPrompts.length} question(s), ${orbit.done.length} approved write(s)`;
	if (!failure && item.ends === "question" && orbit.threads.get(thread)?.waiting_for_user_id === null) failure = "no question";
	orbit.done.length = 0;
	return {
		id: item.id,
		category: item.category,
		success: !failure,
		...(failure ? { failure } : {}),
		requests: mine.length,
		inputTokens: sum(mine.map((stat) => stat.input)),
		outputTokens: sum(mine.map((stat) => stat.output)),
		fixedTokens: mine[0]?.fixed ?? 0,
		codemodeTokens: mine[0]?.codemode ?? 0,
		turns: mine.map((stat) => stat.turn),
		answer,
	};
}

async function main(label: string): Promise<void> {
	const orbit = new FakeOrbit();
	await orbit.listen();
	const dir = mkdtempSync(join(tmpdir(), "donna-prompt-eval-"));
	const socket = join(dir, "sandbox.sock");
	const broker = await fakeBroker(socket);
	const stats: RequestStat[] = [];
	const config = evalConfig(orbit, join(dir, "data"), socket);
	const agent = new Agent(config, new OrbitClient(config), () => new Date("2026-11-15T09:00:00Z"), scriptedModels(config.model, stats));
	await agent.open();
	await agent.start();
	const results: CaseResult[] = [];
	try {
		for (const item of CASES) results.push(await runCase(orbit, stats, item));
	} finally {
		await agent.stop();
		orbit.server.close();
		broker.close();
		rmSync(dir, { recursive: true, force: true });
	}
	const requests = sum(results.map((result) => result.requests));
	const summary = {
		cases: results.length,
		success: results.filter((result) => result.success).length,
		modelRequests: requests,
		/** System prompt with its sections and the tool definitions of a case's first request (tokens = characters / 4). */
		fixedTokensPerRequest: Math.round(sum(results.map((result) => result.fixedTokens)) / results.length),
		codemodeSectionTokens: Math.round(sum(results.map((result) => result.codemodeTokens)) / results.length),
		inputTokensPerRequest: Math.round(sum(results.map((result) => result.inputTokens)) / requests),
		inputTokensTotal: sum(results.map((result) => result.inputTokens)),
		totalTokens: sum(results.map((result) => result.inputTokens + result.outputTokens)),
	};
	const output = { generatedAt: new Date().toISOString(), label, model: "scripted (./scripted-model.ts); tokens = characters / 4", summary, cases: results };
	writeFileSync(new URL(`./results-${label}.json`, import.meta.url), `${JSON.stringify(output, null, "\t")}\n`);
	console.table({ [label]: summary });
	for (const result of results.filter((item) => !item.success)) console.log(`FAIL ${result.id}: ${result.failure}`);
	process.exit(0);
}

if (import.meta.url === `file://${process.argv[1]}`) await main(process.argv[2] ?? "current");
