/**
 * Orbit pages are BlockNote blocks. The agent reads and writes Markdown; this converts the common block
 * kinds both ways. Inline styles become plain text; Orbit's server fills missing block props.
 */

type Inline = { type: string; text?: string; href?: string; content?: Inline[]; props?: { name?: string } };
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
			if (item.type === "link") return `[${inlineText(item.content)}](${item.href ?? ""})`;
			if (item.type === "mention") return `@${item.props?.name ?? ""}`;
			return inlineText(item.content);
		})
		.join("");
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
			default:
				lines.push(`${indent}${text}`);
		}
		if (block.children?.length) lines.push(blocksToMarkdown(block.children, depth + 1));
	}
	return lines.join("\n");
}

const text = (value: string): Inline[] => (value ? [{ type: "text", text: value, styles: {} } as Inline] : []);

/** Markdown → BlockNote blocks: headings, lists, check lists, quotes, code blocks, dividers, paragraphs. */
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
		let match: RegExpExecArray | null;
		if ((match = /^(#{1,6})\s+(.*)$/.exec(trimmed))) {
			blocks.push({ type: "heading", props: { level: Math.min(match[1].length, 3) }, content: text(match[2]) });
		} else if ((match = /^[-*]\s+\[([ xX])\]\s+(.*)$/.exec(trimmed))) {
			blocks.push({ type: "checkListItem", props: { checked: match[1] !== " " }, content: text(match[2]) });
		} else if ((match = /^[-*+]\s+(.*)$/.exec(trimmed))) {
			blocks.push({ type: "bulletListItem", content: text(match[1]) });
		} else if ((match = /^\d+[.)]\s+(.*)$/.exec(trimmed))) {
			blocks.push({ type: "numberedListItem", content: text(match[1]) });
		} else if ((match = /^>\s?(.*)$/.exec(trimmed))) {
			blocks.push({ type: "quote", content: text(match[1]) });
		} else if (/^(-{3,}|\*{3,})$/.test(trimmed)) {
			blocks.push({ type: "divider" });
		} else {
			blocks.push({ type: "paragraph", content: text(trimmed) });
		}
	}
	return blocks;
}
