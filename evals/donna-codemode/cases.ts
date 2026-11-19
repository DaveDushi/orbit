/**
 * The eval's requests. Each case is what a capable model does for the request, as steps of tool calls: the calls of
 * one step run in one model turn, a step can use ids from an earlier step's results (`"$<step>:<regex>"`, group 1 of the
 * first match). The scripted model (./scripted-model.ts) plays the same steps in both modes; only how it reaches a tool
 * changes (a direct call, or a codemode script, after find_tools when it does not know the tool yet). A call of a
 * connected service's tool is `use_integration` with `find`: the words the model searches it with.
 */

import { ANN, BOB } from "./fake-orbit.ts";

export type Call = { tool: string; args: Record<string, unknown>; find?: string };
export type Step = Call[];
export type Category = "one-step" | "multi-step" | "drive-docs" | "image" | "approval" | "mcp";

export type Case = {
	readonly id: string;
	readonly category: Category;
	readonly text: string;
	readonly steps: readonly Step[];
	/** The run ends at the last step (an approval or a question); after `yes` (approvals), the last step runs again. */
	readonly ends?: "approval" | "question";
	/** Chat files sent with the message. */
	readonly files?: readonly { id: string; file_name: string; mime_type: string; size_bytes: number }[];
	/** Every model writes this as a script in both modes (many items): today's prompt asks for codemode there. */
	readonly bulk?: string;
	/** Orbit calls (method, path pattern and, for `/agent-tools/call`, the tool) that must happen as the driver for the case to succeed. */
	readonly expect: readonly ([string, RegExp] | [string, RegExp, string])[];
	/** Only with the connected services' 105 tools. */
	readonly services?: true;
};

/** `use_integration` of `tool` with `args`, found with the words `find`. */
const mcp = (tool: string, args: Record<string, unknown>, find: string): Call => ({ tool: "use_integration", args: { tool, arguments: args }, find });
const CALL = /^\/agent-tools\/call$/;

const TASK_ID = '"id": "([0-9a-f-]{36})"';
const PAGE_ID = '"id": "(0199c3a0-0000-7000-8000-2[0-9a-f]{11})"';
const NODE_ID = '"id": "(0199c3a0-0000-7000-8000-3[0-9a-f]{11})"';
const img = (n: number) => ({ id: `0199c3a0-0000-7000-8000-4000000000${n}0`, file_name: `shot-${n}.png`, mime_type: "image/png", size_bytes: 120_000 });

export const CASES: readonly Case[] = [
	// One step
	{ id: "c01", category: "one-step", text: "Find the tasks about the login bug.", steps: [[{ tool: "search_tasks", args: { query: "login" } }]], expect: [["GET", /^\/tasks$/]] },
	{ id: "c02", category: "one-step", text: 'Create a task "Fix footer links".', steps: [[{ tool: "create_task", args: { title: "Fix footer links" } }]], expect: [["POST", /^\/tasks$/]] },
	{ id: "c03", category: "one-step", text: "Which projects do we have, and their statuses?", steps: [[{ tool: "list_projects", args: {} }]], expect: [["GET", /^\/projects\/p1\/statuses/]] },
	{ id: "c04", category: "one-step", text: "Who is in this workspace?", steps: [[{ tool: "list_members", args: {} }]], expect: [["GET", /^\/members/]] },
	{ id: "c05", category: "one-step", text: "Search the docs for onboarding.", steps: [[{ tool: "search_pages", args: { query: "onboarding" } }]], expect: [["GET", /^\/pages\/search/]] },
	{ id: "c06", category: "one-step", text: "What was said in this thread so far?", steps: [[{ tool: "read_messages", args: {} }]], expect: [["GET", /^\/chat\/threads\//]] },
	{ id: "c07", category: "one-step", text: "Remember that Acme renews its contract in March.", steps: [[{ tool: "memory_note", args: { scope: "team", text: "Acme renews its contract in March." } }]], expect: [["POST", /^\/agent-memory\/team$/]] },
	{ id: "c08", category: "one-step", text: "What do you remember about Acme?", steps: [[{ tool: "memory_recall", args: { scope: "team", query: "Acme" } }]], expect: [["GET", /^\/agent-memory\/team$/]] },
	{ id: "c09", category: "one-step", text: "Which milestones and cycles does Website have?", steps: [[{ tool: "list_milestones_cycles", args: { project_id: "WEB" } }]], expect: [["GET", /^\/projects\/p1\/cycles/]] },
	{ id: "c10", category: "one-step", text: "Find chat messages about the invoice.", steps: [[{ tool: "search_messages", args: { query: "invoice" } }]], expect: [["GET", /^\/chat\/search/]] },
	{ id: "c11", category: "one-step", text: "Remind me today at 17:00 to check the deploy.", steps: [[{ tool: "remind_me", args: { text: "Check the deploy", at: "2026-11-15T17:00:00" } }]], expect: [["POST", /^\/agent-tools\/scheduled$/]] },
	{ id: "c12", category: "one-step", text: "Ask Bob when the deploy happens.", steps: [[{ tool: "ask_teammate", args: { person: "Bob", question: "When does the deploy happen?" } }]], ends: "question", expect: [["GET", /^\/members/]] },
	// Several tools
	{
		id: "c13",
		category: "multi-step",
		text: "Find the Safari login bug and move it to In progress.",
		steps: [[{ tool: "search_tasks", args: { query: "login safari" } }], [{ tool: "update_task", args: { task_id: `$0:${TASK_ID}`, status: "In progress" } }]],
		expect: [["PATCH", /^\/tasks\/[0-9a-f-]{36}$/]],
	},
	{
		id: "c14",
		category: "multi-step",
		text: "Read the footer layout task and comment that I am on it.",
		steps: [[{ tool: "search_tasks", args: { query: "footer" } }], [{ tool: "get_task", args: { task_id: `$0:${TASK_ID}` } }], [{ tool: "comment_on_task", args: { task_id: `$0:${TASK_ID}`, body: "Ann is on it." } }]],
		expect: [["POST", /^\/tasks\/[0-9a-f-]{36}\/comments$/]],
	},
	{
		id: "c15",
		category: "multi-step",
		text: "Make a task for each bug on the Bugs page.",
		steps: [
			[{ tool: "search_pages", args: { query: "bugs" } }],
			[{ tool: "read_page", args: { page_id: `$0:${PAGE_ID}` } }],
			[
				{ tool: "create_task", args: { title: "Checkout button misaligned" } },
				{ tool: "create_task", args: { title: "Search returns stale results" } },
			],
		],
		expect: [["POST", /^\/tasks$/], ["POST", /^\/tasks$/]],
	},
	{
		id: "c16",
		category: "multi-step",
		text: 'Show the label rules of Website, then add one: label "design" goes to Bob.',
		steps: [[{ tool: "list_label_rules", args: { project_id: "WEB" } }], [{ tool: "create_label_rule", args: { project_id: "WEB", label: "design", person: "Bob" } }]],
		expect: [["POST", /^\/projects\/p1\/label-rules$/]],
	},
	{
		id: "c17",
		category: "multi-step",
		text: "Follow every login task for me.",
		steps: [[{ tool: "search_tasks", args: { query: "login" } }], [{ tool: "follow_task", args: { task_id: `$0:${TASK_ID}`, follow: true } }, { tool: "follow_task", args: { task_id: "0199c3a0-0000-7000-8000-100000000003", follow: true } }]],
		expect: [["PUT", /^\/tasks\/.+\/followers\/me$/], ["PUT", /^\/tasks\/.+\/followers\/me$/]],
	},
	{
		id: "c18",
		category: "multi-step",
		text: "Link the Project plan page to the Release v2 task.",
		steps: [[{ tool: "search_pages", args: { query: "plan" } }, { tool: "search_tasks", args: { query: "release" } }], [{ tool: "link_doc_to_task", args: { task_id: "0199c3a0-0000-7000-8000-100000000004", page_id: `$0:${PAGE_ID}` } }]],
		expect: [["POST", /^\/tasks\/.+\/linked-docs$/]],
	},
	{
		id: "c19",
		category: "multi-step",
		text: "Write a page that lists the open login tasks.",
		steps: [[{ tool: "search_tasks", args: { query: "login" } }], [{ tool: "create_page", args: { title: "Open login tasks", markdown: "- WEB-1 Login bug on Safari\n- WEB-3 Login page copy" } }]],
		expect: [["POST", /^\/pages$/]],
	},
	{
		id: "c20",
		category: "multi-step",
		text: "Count our tasks by status.",
		steps: [[{ tool: "search_tasks", args: {} }]],
		bulk: 'const found = await tools.search_tasks({ limit: 50 });\nconst counts = {};\nfor (const match of found.matchAll(/"status": "([^"]+)"/g)) counts[match[1]] = (counts[match[1]] ?? 0) + 1;\ntext(JSON.stringify(counts));',
		expect: [["GET", /^\/tasks$/]],
	},
	// Drive and Docs
	{
		id: "c21",
		category: "drive-docs",
		text: "Find the Q3 budget in Drive and tell me the total.",
		steps: [[{ tool: "drive_search", args: { query: "budget" } }], [{ tool: "drive_read", args: { node_id: `$0:${NODE_ID}` } }]],
		expect: [["POST", /^\/agent-tools\/drive$/], ["POST", /^\/agent-tools\/drive$/]],
	},
	{ id: "c22", category: "drive-docs", text: 'Make a Drive folder "Invoices 2026".', steps: [[{ tool: "drive_create_folder", args: { name: "Invoices 2026" } }]], expect: [["POST", /^\/agent-tools\/drive$/]] },
	{
		id: "c23",
		category: "drive-docs",
		text: "Read the CSV I sent and tell me the cities.",
		files: [{ id: "0199c3a0-0000-7000-8000-500000000001", file_name: "leads.csv", mime_type: "text/csv", size_bytes: 300 }],
		steps: [[{ tool: "read_chat_file", args: { file_id: "0199c3a0-0000-7000-8000-500000000001" } }]],
		expect: [["GET", /^\/chat\/files\/.+\/content/]],
	},
	{
		id: "c24",
		category: "drive-docs",
		text: "Put the Drive file logo.png on the Brand page.",
		steps: [[{ tool: "drive_search", args: { query: "logo" } }, { tool: "search_pages", args: { query: "brand" } }], [{ tool: "insert_drive_file", args: { page_id: "0199c3a0-0000-7000-8000-200000000005", node_id: `$0:${NODE_ID}` } }]],
		expect: [["POST", /^\/pages\/.+\/drive-files$/]],
	},
	{
		id: "c25",
		category: "drive-docs",
		text: 'Add "v2 shipped" to the release notes page.',
		steps: [[{ tool: "search_pages", args: { query: "release" } }], [{ tool: "update_page", args: { page_id: `$0:${PAGE_ID}`, markdown: "v2 shipped.", mode: "append" } }]],
		expect: [["PATCH", /^\/pages\/[0-9a-f-]{36}$/]],
	},
	{
		id: "c26",
		category: "drive-docs",
		text: "Which Drive files are on the Brand page?",
		steps: [[{ tool: "search_pages", args: { query: "brand" } }], [{ tool: "list_page_drive_files", args: { page_id: `$0:${PAGE_ID}` } }]],
		expect: [["GET", /^\/pages\/.+\/drive-files$/]],
	},
	{
		id: "c27",
		category: "drive-docs",
		text: "What does the spreadsheet on the Budget page say?",
		steps: [[{ tool: "search_pages", args: { query: "budget" } }], [{ tool: "read_page", args: { page_id: `$0:${PAGE_ID}` } }], [{ tool: "read_file", args: { page_id: `$0:${PAGE_ID}`, file_id: "f1" } }]],
		expect: [["GET", /^\/pages\/.+\/files\/f1\/content/]],
	},
	// Images
	{ id: "c28", category: "image", text: "What is in this screenshot?", files: [img(1)], steps: [[{ tool: "view_image", args: { chat_file_ids: [img(1).id] } }]], expect: [["GET", /^\/chat\/files\/[^/]+$/]] },
	{ id: "c29", category: "image", text: "Compare these two photos.", files: [img(2), img(3)], steps: [[{ tool: "view_image", args: { chat_file_ids: [img(2).id, img(3).id] } }]], expect: [["GET", /^\/chat\/files\/[^/]+$/], ["GET", /^\/chat\/files\/[^/]+$/]] },
	{
		id: "c30",
		category: "image",
		text: "Make a task from the bug in this screenshot.",
		files: [img(4)],
		steps: [[{ tool: "view_image", args: { chat_file_ids: [img(4).id] } }], [{ tool: "create_task", args: { title: "Checkout button overlaps the cart", description: "From Ann's screenshot." } }]],
		expect: [["POST", /^\/tasks$/]],
	},
	{
		id: "c31",
		category: "image",
		text: "Look at hero.jpg in Drive: is it a landscape photo?",
		steps: [[{ tool: "drive_search", args: { query: "hero" } }], [{ tool: "view_image", args: { node_ids: [`$0:${NODE_ID}`] } }]],
		expect: [["POST", /^\/agent-tools\/drive$/]],
	},
	// Writes that need the driver's yes
	{
		id: "c32",
		category: "approval",
		text: "Move draft-v1.docx in Drive to the trash.",
		steps: [[{ tool: "drive_search", args: { query: "draft" } }], [{ tool: "drive_trash", args: { node_ids: [`$0:${NODE_ID}`] } }]],
		ends: "approval",
		expect: [["POST", /^\/agent-tools\/drive$/]],
	},
	{
		id: "c33",
		category: "approval",
		text: "Make a public link to brochure.pdf.",
		steps: [[{ tool: "drive_search", args: { query: "brochure" } }], [{ tool: "drive_share", args: { node_id: `$0:${NODE_ID}` } }]],
		ends: "approval",
		expect: [["POST", /^\/agent-tools\/drive$/]],
	},
	{
		id: "c34",
		category: "approval",
		text: "Every weekday at 9, post my open tasks here.",
		steps: [[{ tool: "schedule_routine", args: { name: "Open tasks", instruction: "Post Ann's open tasks.", cron: "0 9 * * 1-5" } }]],
		ends: "approval",
		expect: [["POST", /^\/agent-tools\/scheduled$/]],
	},
	{
		id: "c35",
		category: "approval",
		text: 'Tomorrow at 8, post "Standup moved to 10" for me.',
		steps: [[{ tool: "send_later", args: { text: "Standup moved to 10", at: "2026-11-16T08:00:00" } }]],
		ends: "approval",
		expect: [["POST", /^\/agent-tools\/scheduled$/]],
	},
	// Tools of connected services (105 tools: TikTok Ads, TikTok, Shopify, Cloudflare)
	{
		id: "c36",
		category: "mcp",
		services: true,
		text: "How much did we spend on TikTok ads yesterday?",
		steps: [[mcp("tt_ads__get_report", { advertiser_id: "7001", start_date: "2026-11-14", end_date: "2026-11-14", metrics: ["spend"] }, "tiktok ads spend report")]],
		expect: [["POST", CALL, "tt_ads__get_report"]],
	},
	{ id: "c37", category: "mcp", services: true, text: "List our active TikTok ad campaigns.", steps: [[mcp("tt_ads__get_campaigns", { advertiser_id: "7001", status: "ENABLE" }, "tiktok ad campaigns")]], expect: [["POST", CALL, "tt_ads__get_campaigns"]] },
	{ id: "c38", category: "mcp", services: true, text: "Which of our TikTok videos got the most views?", steps: [[mcp("tiktok__list_videos", { max_count: 20 }, "tiktok videos views")]], expect: [["POST", CALL, "tiktok__list_videos"]] },
	{ id: "c39", category: "mcp", services: true, text: "What is the status of Shopify order 1042?", steps: [[mcp("shopify__get_order", { order: "1042" }, "shopify order status")]], expect: [["POST", CALL, "shopify__get_order"]] },
	{
		id: "c40",
		category: "mcp",
		services: true,
		text: "Make a task for the TikTok campaign that is over its budget today.",
		steps: [[mcp("tt_ads__get_campaigns", { advertiser_id: "7001" }, "tiktok ad campaigns budget")], [{ tool: "create_task", args: { title: "Black Friday campaign is over budget" } }]],
		expect: [["POST", CALL, "tt_ads__get_campaigns"], ["POST", /^\/tasks$/]],
	},
	{
		id: "c41",
		category: "mcp",
		services: true,
		text: "Raise the daily budget of the Black Friday campaign to 500.",
		steps: [[mcp("tt_ads__get_campaigns", { advertiser_id: "7001" }, "tiktok ad campaigns")], [mcp("tt_ads__update_campaign_budget", { advertiser_id: "7001", campaign_id: '$0:"campaign_id": "(cmp_\\d+)",\\s*"name": "Black Friday"', budget: 500 }, "change campaign budget")]],
		ends: "approval",
		expect: [["POST", CALL, "tt_ads__update_campaign_budget"]],
	},
	{
		id: "c42",
		category: "mcp",
		services: true,
		text: "Pause the ad group Retargeting EU.",
		steps: [[mcp("tt_ads__get_ad_groups", { advertiser_id: "7001" }, "tiktok ad groups")], [mcp("tt_ads__update_ad_group_status", { advertiser_id: "7001", adgroup_ids: ['$0:"adgroup_id": "(ag_\\d+)",\\s*"name": "Retargeting EU"'], status: "DISABLE" }, "pause ad group status")]],
		ends: "approval",
		expect: [["POST", CALL, "tt_ads__update_ad_group_status"]],
	},
	{
		id: "c43",
		category: "mcp",
		services: true,
		text: "Purge the Cloudflare cache of orbit.test.",
		steps: [[mcp("cloudflare__list_zones", { name: "orbit.test" }, "cloudflare zones domain")], [mcp("cloudflare__purge_cache", { zone_id: '$0:"id": "(zone_\\w+)"', files: [] }, "purge cache")]],
		ends: "approval",
		expect: [["POST", CALL, "cloudflare__purge_cache"]],
	},
];

export const PEOPLE = { ANN, BOB };
