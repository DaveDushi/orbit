/**
 * Runs every case (./cases.ts) against a fake Orbit with the old tool setup (`direct`: the branch base's sidecar, every
 * tool offered, the connected services' tools listed in the prompt) and code mode (`codemode`: this sidecar, only
 * find_tools and codemode), with the scripted model (./scripted-model.ts), once without connected services and once
 * with 105 service tools; measures find_tools' search; writes results.json. Usage: ./run.sh (Node 22.19+).
 */

import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { Config } from "../../apps/agent/src/config.ts";
import { integrationEntry, orbitEntry, rank } from "../../apps/agent/src/find-tools.ts";
import { OrbitClient } from "../../apps/agent/src/orbit.ts";
import { MEDIA_TOOLS, ORBIT_TOOLS } from "../../apps/agent/src/tools.ts";
import { CASES, type Case } from "./cases.ts";
import { ANN, BOT, fakeBroker, FakeOrbit, id, SECRET } from "./fake-orbit.ts";
import { MCP_TOOLS } from "./fake-mcp.ts";
import { scriptedModels, type RequestStat } from "./scripted-model.ts";

type Mode = "direct" | "codemode";
type Scenario = "no-services" | "105-service-tools";
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** The sidecar of a mode: the old one is the branch base's code, unpacked by ./run.sh into .baseline. */
const AGENT = { direct: "./.baseline/apps/agent/src/agent.ts", codemode: "../../apps/agent/src/agent.ts" };

export function evalConfig(orbit: FakeOrbit, dataDir: string, sandboxSocket: string | undefined, maxToolCalls = 40): Config {
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
		maxToolCallsPerRun: maxToolCalls,
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

type CaseResult = {
	id: string;
	category: string;
	success: boolean;
	failure?: string;
	requests: number;
	inputTokens: number;
	uncachedInputTokens: number;
	outputTokens: number;
	maxInputTokensPerRequest: number;
	toolDefTokensPerRequest: number;
	toolsOffered: number;
	modelToolCalls: number;
	turns: string[];
	orbitCalls: number;
	latencyMs: number;
	approvalPrompts: string[];
	progress: string;
	answer: string;
};

async function runMode(mode: Mode, scenario: Scenario): Promise<CaseResult[]> {
	const { Agent } = (await import(AGENT[mode])) as typeof import("../../apps/agent/src/agent.ts");
	const orbit = new FakeOrbit();
	if (scenario === "105-service-tools") orbit.services = MCP_TOOLS;
	await orbit.listen();
	const dir = mkdtempSync(join(tmpdir(), `donna-eval-${mode}-`));
	const socket = join(dir, "sandbox.sock");
	const broker = await fakeBroker(socket);
	const stats: RequestStat[] = [];
	const config = evalConfig(orbit, join(dir, "data"), socket);
	const agent = new Agent(config, new OrbitClient(config), () => new Date("2026-11-15T09:00:00Z"), scriptedModels(config.model, stats));
	await agent.open();
	await agent.start();
	const results: CaseResult[] = [];
	try {
		for (const item of CASES.filter((candidate) => scenario === "105-service-tools" || !candidate.services)) results.push(await runCase(orbit, stats, item));
	} finally {
		await agent.stop();
		orbit.server.close();
		broker.close();
		rmSync(dir, { recursive: true, force: true });
	}
	return results;
}

async function runCase(orbit: FakeOrbit, stats: RequestStat[], item: Case): Promise<CaseResult> {
	const thread = id();
	const firstCall = orbit.calls.length;
	const firstStat = stats.length;
	const approvalsBefore = new Set([...orbit.messages.keys()].filter((key) => key.startsWith("approve:")));
	const started = performance.now();
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
	const latencyMs = performance.now() - started;
	const calls = orbit.calls.slice(firstCall).filter((call) => call.as === ANN);
	const mine = stats.slice(firstStat).filter((stat) => stat.caseId === item.id);
	const approvalPrompts = [...orbit.messages.entries()].filter(([key]) => key.startsWith("approve:") && !approvalsBefore.has(key)).map(([, message]) => message.body);
	const answer = [first, ...(last === first ? [] : [last])].map((messageId) => orbit.messages.get(`agent:${messageId}`)?.body ?? "").join("\n---\n");
	if (!failure) {
		const remaining = [...calls];
		for (const [method, path, tool] of item.expect) {
			const index = remaining.findIndex((call) => call.method === method && path.test(call.path) && (!tool || (call.body as { tool?: string } | undefined)?.tool === tool));
			if (index < 0) {
				failure = `missing ${method} ${path}${tool ? ` ${tool}` : ""}`;
				break;
			}
			remaining.splice(index, 1);
		}
	}
	if (!failure && /a tool failed|I could not finish|no case/.test(answer)) failure = `answer: ${answer.slice(0, 300)}`;
	if (!failure && item.ends === "approval" && (approvalPrompts.length !== 1 || !orbit.done.length)) failure = `approval: ${approvalPrompts.length} question(s), ${orbit.done.length} approved write(s)`;
	if (!failure && item.ends === "question" && orbit.threads.get(thread)?.waiting_for_user_id === null) failure = "no question";
	orbit.done.length = 0;
	const footers = answer.match(/\n\n_[^\n]*_$/gm) ?? [];
	return {
		id: item.id,
		category: item.category,
		success: !failure,
		...(failure ? { failure } : {}),
		requests: mine.length,
		inputTokens: sum(mine.map((stat) => stat.input)),
		uncachedInputTokens: sum(mine.map((stat) => stat.input - stat.cached)),
		outputTokens: sum(mine.map((stat) => stat.output)),
		maxInputTokensPerRequest: Math.max(0, ...mine.map((stat) => stat.input)),
		toolDefTokensPerRequest: mine[0]?.toolDefs ?? 0,
		toolsOffered: mine[0]?.tools ?? 0,
		modelToolCalls: mine.filter((stat) => stat.turn !== "answer").length,
		turns: mine.map((stat) => stat.turn),
		orbitCalls: calls.length,
		latencyMs: Math.round(latencyMs),
		approvalPrompts,
		progress: footers.map((footer) => footer.trim()).join(" | "),
		answer,
	};
}

/** Assumed real-model timing (not measured): time to first token, prefill of uncached input, output speed. */
const MODELED_LATENCY = { perRequestSec: 0.8, inputTokensPerSec: 10_000, outputTokensPerSec: 60 };

const sum = (values: number[]) => values.reduce((total, value) => total + value, 0);
const mean = (values: number[]) => (values.length ? sum(values) / values.length : 0);
const median = (values: number[]) => {
	const sorted = [...values].sort((a, b) => a - b);
	return sorted.length ? sorted[Math.floor(sorted.length / 2)] : 0;
};

function summary(results: CaseResult[]) {
	const requests = sum(results.map((result) => result.requests));
	return {
		cases: results.length,
		successRate: results.filter((result) => result.success).length / results.length,
		modelRequests: requests,
		modelRequestsPerCase: requests / results.length,
		inputTokensPerRequest: sum(results.map((result) => result.inputTokens)) / requests,
		toolDefTokensPerRequest: mean(results.map((result) => result.toolDefTokensPerRequest)),
		toolsOfferedPerRequest: mean(results.map((result) => result.toolsOffered)),
		inputTokensTotal: sum(results.map((result) => result.inputTokens)),
		uncachedInputTokensTotal: sum(results.map((result) => result.uncachedInputTokens)),
		outputTokensTotal: sum(results.map((result) => result.outputTokens)),
		totalTokens: sum(results.map((result) => result.inputTokens + result.outputTokens)),
		/** Sidecar time with an instant model: Pi, the tools, the QuickJS sandbox and the fake Orbit. */
		latencyMsMedian: median(results.map((result) => result.latencyMs)),
		/** With a real model, per MODELED_LATENCY: each request's wait, its uncached input, its output. */
		modeledLatencySecPerCase: (requests * MODELED_LATENCY.perRequestSec + sum(results.map((result) => result.uncachedInputTokens / MODELED_LATENCY.inputTokensPerSec + result.outputTokens / MODELED_LATENCY.outputTokensPerSec + result.latencyMs / 1000))) / results.length,
		latencyMsTotal: sum(results.map((result) => result.latencyMs)),
		approvalPrompts: sum(results.map((result) => result.approvalPrompts.length)),
	};
}

/** Requests in people's words and the tool find_tools should return for them (`mcp:` a connected service's). */
const SEARCHES: [string, string][] = [
	["move a drive file to the trash", "drive_trash"],
	["public link for a file", "drive_share"],
	["every weekday post a summary", "schedule_routine"],
	["remind me tomorrow", "remind_me"],
	["post a message later", "send_later"],
	["who is in the workspace", "list_members"],
	["search chat messages", "search_messages"],
	["read a csv someone sent in chat", "read_chat_file"],
	["label rule for a person", "create_label_rule"],
	["milestones and cycles of a project", "list_milestones_cycles"],
	["follow a task", "follow_task"],
	["put a drive file on a page", "insert_drive_file"],
	["create a drive folder", "drive_create_folder"],
	["trash a page and a task", "move_to_trash"],
	["save this script", "save_script"],
	["tiktok ads spend yesterday", "mcp:tt_ads__get_report"],
	["active tiktok campaigns", "mcp:tt_ads__get_campaigns"],
	["raise the campaign budget", "mcp:tt_ads__update_campaign_budget"],
	["pause an ad group", "mcp:tt_ads__update_ad_group_status"],
	["turn off an ad", "mcp:tt_ads__update_ad_status"],
	["lookalike audience", "mcp:tt_ads__create_lookalike_audience"],
	["pixel purchases events", "mcp:tt_ads__get_pixel_events"],
	["upload a video to the ad library", "mcp:tt_ads__upload_video"],
	["comments on our ads", "mcp:tt_ads__get_comments"],
	["our tiktok videos with the most views", "mcp:tiktok__list_videos"],
	["tiktok followers by country", "mcp:tiktok__get_follower_demographics"],
	["post a video on tiktok", "mcp:tiktok__publish_video"],
	["shopify order status", "mcp:shopify__get_order"],
	["refund an order", "mcp:shopify__refund_order"],
	["product stock level", "mcp:shopify__set_inventory"],
	["discount code", "mcp:shopify__create_discount"],
	["sales by day", "mcp:shopify__get_sales_report"],
	["purge the cdn cache", "mcp:cloudflare__purge_cache"],
	["add a dns record", "mcp:cloudflare__create_dns_record"],
	["block an ip address", "mcp:cloudflare__block_ip"],
	["website traffic analytics cloudflare", "mcp:cloudflare__get_analytics"],
];

/** find_tools' ranking over every Orbit tool and the 105 service tools: where the expected tool lands. */
function searchQuality() {
	const entries = [...[...ORBIT_TOOLS, ...MEDIA_TOOLS].map(orbitEntry), ...MCP_TOOLS.map(integrationEntry)];
	const positions = SEARCHES.map(([query, wanted]) => ({ query, wanted, position: rank(query, entries).indexOf(wanted) + 1 || null }));
	const share = (top: number) => positions.filter((item) => item.position !== null && item.position <= top).length / positions.length;
	return { tools: entries.length, queries: positions.length, top1: share(1), top3: share(3), top8: share(8), misses: positions.filter((item) => item.position !== 1) };
}

async function main(): Promise<void> {
	const scenarios: Scenario[] = ["no-services", "105-service-tools"];
	const runs: Record<string, { direct: CaseResult[]; codemode: CaseResult[] }> = {};
	for (const scenario of scenarios) runs[scenario] = { direct: await runMode("direct", scenario), codemode: await runMode("codemode", scenario) };
	const categories = (results: CaseResult[]) => [...new Set(results.map((result) => result.category))];
	const byCategory = (results: CaseResult[]) => Object.fromEntries(categories(results).map((category) => [category, summary(results.filter((result) => result.category === category))]));
	const compare = ({ direct, codemode }: { direct: CaseResult[]; codemode: CaseResult[] }) => {
		const same = direct.map((old, index) => ({
			id: old.id,
			approvalPromptsSame: JSON.stringify(old.approvalPrompts) === JSON.stringify(codemode[index].approvalPrompts),
			progressSame: old.progress === codemode[index].progress,
			...(old.progress === codemode[index].progress ? {} : { progressDirect: old.progress, progressCodemode: codemode[index].progress }),
		}));
		return { approvalPromptsSame: same.filter((item) => item.approvalPromptsSame).length, progressSame: same.filter((item) => item.progressSame).length, of: same.length, differences: same.filter((item) => !item.approvalPromptsSame || !item.progressSame) };
	};
	const output = {
		generatedAt: new Date().toISOString(),
		model: "scripted (evals/donna-codemode/scripted-model.ts); tokens = characters / 4",
		baseline: process.env.BASELINE ?? "see run.sh",
		modeledLatency: MODELED_LATENCY,
		search: searchQuality(),
		summary: Object.fromEntries(scenarios.map((scenario) => [scenario, { direct: summary(runs[scenario].direct), codemode: summary(runs[scenario].codemode) }])),
		byCategory: Object.fromEntries(scenarios.map((scenario) => [scenario, { direct: byCategory(runs[scenario].direct), codemode: byCategory(runs[scenario].codemode) }])),
		userVisible: Object.fromEntries(scenarios.map((scenario) => [scenario, compare(runs[scenario])])),
		cases: runs,
	};
	writeFileSync(new URL("./results.json", import.meta.url), `${JSON.stringify(output, null, "\t")}\n`);
	for (const scenario of scenarios) {
		console.log(`\n${scenario}`);
		console.table(output.summary[scenario]);
		for (const mode of ["direct", "codemode"] as const) for (const result of runs[scenario][mode].filter((item) => !item.success)) console.log(`FAIL ${mode} ${result.id}: ${result.failure}`);
	}
	const { misses, ...search } = output.search;
	console.log("find_tools search", search, misses);
	process.exit(0);
}

if (import.meta.url === `file://${process.argv[1]}`) await main();
