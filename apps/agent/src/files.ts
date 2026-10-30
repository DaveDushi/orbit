/**
 * Page files for the model: a read of one part of a file becomes text that fits in one tool result (`untrusted` cuts
 * at 12,000 characters), and says where the next part starts.
 */

/** Text from Orbit content is data for the model, never instructions. The content cannot close its own block. */
export function untrusted(label: string, value: unknown): string {
	const text = typeof value === "string" ? value : JSON.stringify(value, null, 1);
	const bounded = text.length > 12_000 ? `${text.slice(0, 12_000)}\n… (cut)` : text;
	return `<orbit-data source="${label}">\n${bounded.replace(/<\/orbit-data/gi, "<\\/orbit-data")}\n</orbit-data>`;
}

export const READ_CHARS = 10_000;

export type FileContent = {
	file: { id: string; file_name: string; kind: string; version: number; can_edit: boolean; size_bytes: number };
	sheets: string[];
	sheet: string | null;
	rows: string[][] | null;
	row_numbers: number[] | null;
	text: string | null;
	next_offset: number | null;
	no_text: boolean;
};

/** One part of a file as text for the model: what the file is, then its rows or text. */
export function fileReadText(content: FileContent, offset: number): string {
	const { file } = content;
	const about: Record<string, unknown> = { file_id: file.id, name: file.file_name, kind: file.kind, version: file.version, can_write: file.can_edit };
	if (content.sheets.length) Object.assign(about, { sheets: content.sheets, sheet: content.sheet });
	if (file.kind === "image" || file.kind === "other") {
		return `${JSON.stringify(about)}\nThis kind of file has no text you can read. People can open or download it on the page.`;
	}
	if (content.no_text) return `${JSON.stringify(about)}\nNo text was found in this file. It may be a scan; there is no OCR, so you cannot read it.`;
	if (content.rows) {
		const lines: string[] = [];
		let size = 0;
		let next = content.next_offset;
		for (const [index, row] of content.rows.entries()) {
			const line = `row ${content.row_numbers?.[index] ?? offset + index + 1}: ${row.join(" | ")}`;
			// Always at least one row, so a very long row cannot stop the reading.
			const fitted = lines.length === 0 ? line.slice(0, READ_CHARS) : line;
			if (lines.length > 0 && size + fitted.length > READ_CHARS) {
				next = offset + index;
				break;
			}
			lines.push(fitted);
			size += fitted.length + 1;
		}
		return `${JSON.stringify({ ...about, next_offset: next })}\n${lines.join("\n")}`;
	}
	return `${JSON.stringify({ ...about, next_offset: content.next_offset })}\n${content.text ?? ""}`;
}
