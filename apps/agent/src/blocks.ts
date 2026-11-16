/**
 * Orbit pages are BlockNote blocks. The agent reads and writes Markdown; this converts the common block
 * kinds both ways, with GFM tables and links in the shape of the server's Markdown import
 * (`collab/markdown_import.rs`). Other inline styles become plain text; Orbit's server fills missing
 * block props and drops links with other schemes.
 */

type Inline = { type: string; text?: string; href?: string; content?: Inline[]; props?: { name?: string } };
type Cell = { type?: string; props?: { textAlignment?: string }; content?: unknown };

/** Link schemes the agent writes; the server and the editor drop the others too. */
const SAFE_LINK = /^(https?:\/\/|mailto:)/i;
/** A whole bare URL as `inlineContent` finds it: a link whose text is its URL is written back this way. */
const BARE_URL = /^https?:\/\/[^\s<>()[\]]*[^\s<>()[\].,;:!?'"*]$/i;

/** A link destination that stays inside `(…)`, as the server's exporter writes it. */
const destination = (href: string): string =>
	href.replace(/[ ()<>]/g, (character) => `%${character.charCodeAt(0).toString(16).toUpperCase()}`);

export type Block = {
	id?: string;
	type: string;
	props?: Record<string, unknown>;
	content?: Inline[] | string | unknown;
	children?: Block[];
};

function inlineText(content: unknown): string {
	if (typeof content === "string") return content;
	if (!Array.isArray(content)) return "";
	return (content as Inline[])
		.map((item) => {
			if (item.type === "text") return item.text ?? "";
			if (item.type === "link") {
				const label = inlineText(item.content);
				const href = item.href ?? "";
				return label === href && BARE_URL.test(href) ? href : `[${label}](${destination(href)})`;
			}
			if (item.type === "mention") return `@${item.props?.name ?? ""}`;
			return inlineText(item.content);
		})
		.join("");
}

/** A table block → GFM: the first row is the header, column alignment comes from its cells. */
function tableMarkdown(content: unknown): string[] {
	const rows = ((content as { rows?: { cells?: unknown[] }[] } | undefined)?.rows ?? []).map((row) => row.cells ?? []);
	const columns = Math.max(0, ...rows.map((row) => row.length));
	if (!columns) return [];
	// A cell is a `tableCell`, or (older content) its inline content alone.
	const cell = (value: unknown): Cell =>
		value && typeof value === "object" && !Array.isArray(value) && (value as Cell).type === "tableCell" ? (value as Cell) : { content: value };
	const line = (row: unknown[]) =>
		`| ${Array.from({ length: columns }, (_, index) => inlineText(cell(row[index]).content).replace(/\|/g, "\\|").replace(/\n/g, "<br>")).join(" | ")} |`;
	const align = (value: unknown) => ({ center: ":---:", right: "---:" } as Record<string, string>)[cell(value).props?.textAlignment ?? ""] ?? "---";
	return [line(rows[0]), `| ${Array.from({ length: columns }, (_, index) => align(rows[0][index])).join(" | ")} |`, ...rows.slice(1).map(line)];
}

/** BlockNote blocks → Markdown. Unknown blocks keep their text. */
export function blocksToMarkdown(blocks: readonly Block[], depth = 0): string {
	const lines: string[] = [];
	const indent = "  ".repeat(depth);
	let number = 0;
	for (const block of blocks) {
		const text = inlineText(block.content);
		number = block.type === "numberedListItem" ? number + 1 : 0;
		switch (block.type) {
			case "heading":
				lines.push(`${"#".repeat(Math.min(Number(block.props?.level ?? 1), 6))} ${text}`);
				break;
			case "bulletListItem":
			case "toggleListItem":
				lines.push(`${indent}- ${text}`);
				break;
			case "numberedListItem":
				lines.push(`${indent}${number}. ${text}`);
				break;
			case "checkListItem":
				lines.push(`${indent}- [${block.props?.checked ? "x" : " "}] ${text}`);
				break;
			case "codeBlock":
				lines.push(`\`\`\`${block.props?.language && block.props.language !== "text" ? block.props.language : ""}`, text, "```");
				break;
			case "quote":
				lines.push(`> ${text}`);
				break;
			case "divider":
				lines.push("---");
				break;
			case "table":
				lines.push(...tableMarkdown(block.content));
				break;
			case "file":
				lines.push(`${indent}[file: ${block.props?.name ?? "unnamed"}]`);
				break;
			case "driveFile":
				// Only ids: the name needs a check of the reader's Drive access (list_page_drive_files).
				lines.push(`${indent}[Drive file ${block.props?.nodeId ?? ""} (block ${block.id ?? ""})]`);
				break;
			default:
				lines.push(`${indent}${text}`);
		}
		if (block.children?.length) lines.push(blocksToMarkdown(block.children, depth + 1));
	}
	return lines.join("\n");
}

const text = (value: string): Inline[] => (value ? [{ type: "text", text: value, styles: {} } as Inline] : []);

const LINK = /\[([^\]]*)\]\(([^)\s]*)\)|<((?:https?:\/\/|mailto:)[^>\s]+)>|(https?:\/\/[^\s<>()[\]]*[^\s<>()[\].,;:!?'"*])/gi;

/** Inline Markdown → text and links: `[label](url)`, `<url>` and bare http(s) URLs. A link with another scheme stays text. */
function inlineContent(value: string): Inline[] {
	const out: Inline[] = [];
	let last = 0;
	for (const match of value.matchAll(LINK)) {
		const [whole, label, href, angle, bare] = match;
		const url = href ?? angle ?? bare;
		if (!SAFE_LINK.test(url)) continue;
		out.push(...text(value.slice(last, match.index)), { type: "link", href: url, content: text(label || url) });
		last = match.index + whole.length;
	}
	return [...out, ...text(value.slice(last))];
}

const DELIMITER_ROW = /^\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?$/;

/** The cells of a GFM table row; `\|` is a pipe inside a cell. */
function rowCells(row: string): string[] {
	let body = row.trim();
	if (body.startsWith("|")) body = body.slice(1);
	if (body.endsWith("|") && !body.endsWith("\\|")) body = body.slice(0, -1);
	return body.split(/(?<!\\)\|/).map((cell) => cell.trim().replace(/\\\|/g, "|"));
}

/** A GFM table at `lines[index]` (header, delimiter row, body rows until a line without a pipe), or none. */
function tableAt(lines: string[], index: number): { block: Block; end: number } | undefined {
	const delimiter = lines[index + 1]?.trim() ?? "";
	if (!lines[index].includes("|") || !DELIMITER_ROW.test(delimiter)) return undefined;
	const header = rowCells(lines[index]);
	const marks = rowCells(delimiter);
	if (header.length !== marks.length) return undefined;
	const alignments = marks.map((mark) => (mark.startsWith(":") && mark.endsWith(":") ? "center" : mark.endsWith(":") ? "right" : undefined));
	const rows = [header];
	let end = index + 1;
	while (end + 1 < lines.length && lines[end + 1].trim() && lines[end + 1].includes("|")) rows.push(rowCells(lines[++end]));
	const cell = (value: string, column: number) => ({
		type: "tableCell",
		props: alignments[column] ? { textAlignment: alignments[column] } : {},
		content: inlineContent(value.replace(/<br\s*\/?>/gi, "\n")),
	});
	const content = {
		type: "tableContent",
		columnWidths: header.map(() => null),
		headerRows: 1,
		rows: rows.map((row) => ({ cells: header.map((_, column) => cell(row[column] ?? "", column)) })),
	};
	return { block: { type: "table", content }, end };
}

/** Markdown → BlockNote blocks: headings, lists, check lists, quotes, code blocks, dividers, GFM tables, paragraphs, and links in their text. */
export function markdownToBlocks(markdown: string): Block[] {
	const blocks: Block[] = [];
	const lines = markdown.replace(/\r\n/g, "\n").split("\n");
	for (let index = 0; index < lines.length; index++) {
		const line = lines[index];
		const fence = /^```\s*([\w-]*)\s*$/.exec(line);
		if (fence) {
			const code: string[] = [];
			while (index + 1 < lines.length && !/^```\s*$/.test(lines[index + 1])) code.push(lines[++index]);
			index++;
			blocks.push({ type: "codeBlock", props: { language: fence[1] || "text" }, content: code.join("\n") });
			continue;
		}
		const trimmed = line.trim();
		if (!trimmed) continue;
		const table = tableAt(lines, index);
		if (table) {
			blocks.push(table.block);
			index = table.end;
			continue;
		}
		let match: RegExpExecArray | null;
		if ((match = /^(#{1,6})\s+(.*)$/.exec(trimmed))) {
			blocks.push({ type: "heading", props: { level: Math.min(match[1].length, 3) }, content: inlineContent(match[2]) });
		} else if ((match = /^[-*]\s+\[([ xX])\]\s+(.*)$/.exec(trimmed))) {
			blocks.push({ type: "checkListItem", props: { checked: match[1] !== " " }, content: inlineContent(match[2]) });
		} else if ((match = /^[-*+]\s+(.*)$/.exec(trimmed))) {
			blocks.push({ type: "bulletListItem", content: inlineContent(match[1]) });
		} else if ((match = /^\d+[.)]\s+(.*)$/.exec(trimmed))) {
			blocks.push({ type: "numberedListItem", content: inlineContent(match[1]) });
		} else if ((match = /^>\s?(.*)$/.exec(trimmed))) {
			blocks.push({ type: "quote", content: inlineContent(match[1]) });
		} else if (/^(-{3,}|\*{3,})$/.test(trimmed)) {
			blocks.push({ type: "divider" });
		} else {
			blocks.push({ type: "paragraph", content: inlineContent(trimmed) });
		}
	}
	return blocks;
}
