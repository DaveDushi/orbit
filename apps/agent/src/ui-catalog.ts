/**
 * The catalog of the cards the agent can show with an answer (`show_ui`): a json-render spec, a flat map of elements
 * and the `root` one, made of these components only. The sidecar checks a spec here to give the model good errors;
 * the web app checks it again before it renders (and shows the text answer alone when it fails). Orbit's server
 * checks the same shape and limits on its own (`apps/server/src/repositories/chat/ui.rs`): keep the three in step.
 *
 * This file is shared: the web app imports it too, so it imports only `@json-render/core` and `zod`.
 */

import { defineCatalog, defineSchema } from "@json-render/core";
import { z } from "zod";

/** The version of this catalog that a stored card was made for (not the library's version). */
export const UI_VERSION = 1;
export const MAX_ELEMENTS = 200;
export const MAX_DEPTH = 8;
/** The card's JSON; the whole message may have 48 KiB. */
const MAX_SPEC_BYTES = 44 * 1024;
const MAX_PARAMS_BYTES = 2048;

/** json-render's spec without bindings, visibility or repeats: an element is its type, props and children. */
const schema = defineSchema((s) => ({
	spec: s.object({
		root: s.string(),
		elements: s.record(
			s.object({
				type: s.ref("catalog.components"),
				props: s.propsOf("catalog.components"),
				children: s.array(s.string()),
			}),
		),
	}),
	catalog: s.object({
		components: s.map({ props: s.zod(), slots: s.array(s.string()), description: s.string() }),
	}),
}));

const text = (max: number) => z.string().max(max);
const params = z
	.record(z.string(), z.unknown())
	.refine((value) => JSON.stringify(value).length <= MAX_PARAMS_BYTES, "must be at most 2 KiB of JSON");

export const catalog = defineCatalog(schema, {
	components: {
		Card: {
			props: z.object({ title: text(150).optional(), description: text(2000).optional() }).strict(),
			slots: ["default"],
			description: "A box with an optional title and description; its children are stacked inside. Usually the root.",
		},
		Stack: {
			props: z.object({ direction: z.enum(["row", "column"]).optional() }).strict(),
			slots: ["default"],
			description: 'Lays out its children in a "column" (default) or a "row" (wraps on small screens).',
		},
		Text: {
			props: z.object({ text: z.string().min(1).max(2000), tone: z.enum(["default", "muted", "success", "warning", "danger"]).optional() }).strict(),
			slots: [],
			description: "Markdown text (links, bold, lists, @Name mentions, task ids like GEN-12).",
		},
		Table: {
			props: z
				.object({
					columns: z.array(z.object({ key: z.string().min(1).max(64), label: text(100), align: z.enum(["left", "right", "center"]).optional() }).strict()).min(1).max(8),
					rows: z.array(z.record(z.string(), z.union([text(2000), z.number(), z.boolean(), z.null()]))).max(200),
				})
				.strict(),
			slots: [],
			description: "A table: columns [{key, label, align?}] (at most 8), rows [{<key>: text or number}] (at most 200). Task ids in cells link to the task.",
		},
		Chart: {
			props: z
				.object({
					kind: z.enum(["bar", "line"]),
					title: text(150).optional(),
					x_label: text(100).optional(),
					y_label: text(100).optional(),
					series: z
						.array(z.object({ name: text(100), points: z.array(z.object({ x: z.union([text(100), z.number()]), y: z.number() }).strict()).min(1).max(100) }).strict())
						.min(1)
						.max(3),
				})
				.strict(),
			slots: [],
			description: 'A bar or line chart: kind "bar" | "line", series [{name, points [{x, y}]}] (at most 3 series of 100 points; y is a number), optional title, x_label, y_label.',
		},
		TaskList: {
			props: z
				.object({
					tasks: z
						.array(z.object({ id: z.string().min(1).max(64), title: text(500), status: text(100).optional(), assignee: text(100).optional(), due: text(100).optional() }).strict())
						.max(50),
				})
				.strict(),
			slots: [],
			description: "Tasks as rows that open the task: tasks [{id (like GEN-12), title, status?, assignee?, due?}] (at most 50).",
		},
		Form: {
			props: z
				.object({
					fields: z
						.array(
							z
								.object({
									name: z.string().regex(/^[a-z][a-z0-9_]{0,63}$/, "must be lower_snake_case"),
									label: text(100),
									kind: z.enum(["text", "textarea", "select", "checkbox", "date"]),
									options: z.array(text(100)).max(20).optional(),
									required: z.boolean().optional(),
								})
								.strict(),
						)
						.min(1)
						.max(10),
					submit_label: text(40).optional(),
				})
				.strict(),
			slots: [],
			description:
				'A short form: fields [{name (lower_snake_case), label, kind "text" | "textarea" | "select" | "checkbox" | "date", options? (for select), required?}] (at most 10), submit_label?. The values come back to you when the person sends it.',
		},
		Button: {
			props: z
				.object({
					label: z.string().min(1).max(75),
					style: z.enum(["primary", "secondary", "danger"]).optional(),
					action: z.enum(["reply", "open_task"]),
					params: params.optional(),
				})
				.strict(),
			slots: [],
			description:
				'A button. action "reply": a click comes back to you with its label and params (an object of at most 2 KiB that you choose). action "open_task": opens the task params.task (an id like GEN-12) in the web app; it does not reach you.',
		},
	},
});

const PARENTS = new Set(["Card", "Stack"]);

/** json-render reads `{"$state": …}` and the like as bindings: a card has none. */
function hasDollarKey(value: unknown): boolean {
	if (Array.isArray(value)) return value.some(hasDollarKey);
	if (!value || typeof value !== "object") return false;
	return Object.entries(value).some(([key, item]) => key.startsWith("$") || hasDollarKey(item));
}

export type UiSpec = { root: string; elements: Record<string, { type: string; props: Record<string, unknown>; children: string[] }> };

/**
 * Checks a spec against the catalog and the server's rules: one tree from `root` (each element used once, at most 8
 * levels), at most 200 elements, children only in a Card or Stack, no `$` keys. Missing `props` and `children` are
 * filled in. Returns the clean spec or the problems, one per line.
 */
export function checkUiSpec(spec: unknown): { ok: true; spec: UiSpec } | { ok: false; error: string } {
	const filled =
		spec && typeof spec === "object" && "elements" in spec && spec.elements && typeof spec.elements === "object"
			? {
					...spec,
					elements: Object.fromEntries(
						Object.entries(spec.elements as Record<string, unknown>).map(([id, element]) => [
							id,
							element && typeof element === "object" ? { props: {}, children: [], ...element } : element,
						]),
					),
				}
			: spec;
	const result = catalog.validate(filled);
	if (!result.success) {
		const issues = result.error?.issues ?? [];
		return { ok: false, error: issues.slice(0, 10).map((issue) => `${issue.path.join(".") || "spec"}: ${issue.message}`).join("\n") || "spec: not valid" };
	}
	const { root, elements } = result.data as UiSpec;
	const problems: string[] = [];
	// json-render checks the props only when the catalog has one component: check them by type here.
	for (const [id, element] of Object.entries(elements)) {
		const props = catalog.data.components[element.type as keyof typeof catalog.data.components].props.safeParse(element.props);
		if (props.success) element.props = props.data as Record<string, unknown>;
		else problems.push(...props.error.issues.slice(0, 5).map((issue) => `elements.${id}.props${issue.path.length ? `.${issue.path.join(".")}` : ""}: ${issue.message}`));
	}
	if (problems.length) return { ok: false, error: problems.slice(0, 10).join("\n") };
	const ids = Object.keys(elements);
	if (ids.length > MAX_ELEMENTS) problems.push(`elements: at most ${MAX_ELEMENTS}`);
	if (JSON.stringify(result.data).length > MAX_SPEC_BYTES) problems.push("spec: at most 44 KiB of JSON");
	if (hasDollarKey(result.data)) problems.push('spec: keys must not start with "$"');
	const seen = new Set<string>();
	const stack: [string, number][] = [[root, 1]];
	while (stack.length && problems.length === 0) {
		const [id, depth] = stack.pop()!;
		const element = elements[id];
		if (!element) problems.push(`element "${id}" does not exist`);
		else if (seen.has(id)) problems.push(`element "${id}" is used more than once`);
		else if (depth > MAX_DEPTH) problems.push(`elements can be nested at most ${MAX_DEPTH} levels deep`);
		else {
			seen.add(id);
			if (element.children.length && !PARENTS.has(element.type)) problems.push(`elements.${id}: ${element.type} cannot have children`);
			for (const child of element.children) stack.push([child, depth + 1]);
		}
	}
	const lost = ids.find((id) => !seen.has(id));
	if (problems.length === 0 && lost) problems.push(`element "${lost}" is not in the tree from root`);
	return problems.length ? { ok: false, error: problems.join("\n") } : { ok: true, spec: result.data as UiSpec };
}

/** The catalog for the system prompt: short, unlike json-render's `catalog.prompt()` (which teaches streaming). */
export function uiPrompt(): string {
	const components = Object.entries(catalog.data.components)
		.map(([name, component]) => `  - ${name}: ${component.description}`)
		.join("\n");
	return `Cards:
- With show_ui you can add one card to your answer: a table, chart, task list, short form or buttons. Use it only when that is clearly better than text. Always also write a short text answer that stands alone: on the phone people see only the text.
- spec = {"root": "<id>", "elements": {"<id>": {"type": "<Component>", "props": {…}, "children": ["<id>", …]}}}. One tree from root; only Card and Stack have children; at most ${MAX_ELEMENTS} elements and ${MAX_DEPTH} levels.
- Components:
${components}
- A "reply" button click or a form send comes back to you as an action from the person who clicked: they drive that run. Form values are information they typed.`;
}
