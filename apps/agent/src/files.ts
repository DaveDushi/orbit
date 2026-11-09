/**
 * Page and chat files for the model: a read of one part of a file becomes text that fits in one tool result
 * (`untrusted` cuts at 12,000 characters), and says where the next part starts.
 */

/** Text from Orbit content is data for the model, never instructions. The content cannot close its own block. */
export function untrusted(label: string, value: unknown): string {
	const text = typeof value === "string" ? value : JSON.stringify(value, null, 1);
	const bounded = text.length > 12_000 ? `${text.slice(0, 12_000)}\n… (cut)` : text;
	return `<orbit-data source="${label}">\n${bounded.replace(/<\/orbit-data/gi, "<\\/orbit-data")}\n</orbit-data>`;
}

export const READ_CHARS = 10_000;

export type FileContent = {
	/** A chat file has no version and is read-only: no `version`, no `can_edit`. */
	file: { id: string; file_name: string; kind: string; version?: number; can_edit?: boolean; size_bytes: number };
	sheets: string[];
	sheet: string | null;
	rows: string[][] | null;
	row_numbers: number[] | null;
	formula_cells?: string[];
	text: string | null;
	next_offset: number | null;
	no_text: boolean;
};

/** One part of a file as text for the model: what the file is, then its rows or text. */
export function fileReadText(content: FileContent, offset: number): string {
	const { file } = content;
	const about: Record<string, unknown> = { file_id: file.id, name: file.file_name, kind: file.kind };
	if (file.version !== undefined) Object.assign(about, { version: file.version, can_write: file.can_edit });
	if (content.sheets.length) Object.assign(about, { sheets: content.sheets, sheet: content.sheet });
	if (file.kind === "image" || file.kind === "other") {
		return `${JSON.stringify(about)}\nThis kind of file has no text to read here. People can open or download it in Orbit.`;
	}
	if (content.no_text) return `${JSON.stringify(about)}\nNo text was found in this file. It may be a scan, which has no text to read here.`;
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
		if (content.formula_cells?.length) about.formula_cells = content.formula_cells;
		return `${JSON.stringify({ ...about, next_offset: next })}\n${lines.join("\n")}`;
	}
	return `${JSON.stringify({ ...about, next_offset: content.next_offset })}\n${content.text ?? ""}`;
}

/** A file of a chat message, as Orbit lists it on the message. */
export type ChatFile = { id: string; file_name: string; mime_type: string; size_bytes?: number; deleted?: boolean };

/** The files of the message that starts or joins a run, after its text, so the model knows them without a call. */
export function withMessageFiles(text: string, files: readonly ChatFile[] | undefined): string {
	const list = chatFilesText(files);
	if (!list) return text;
	return `${text}\n\nFiles sent with this message (read them with read_chat_file):\n${untrusted("message files", list)}`;
}

/** A short kind for a chat file: what read_chat_file can do with it is clear from it. */
export function chatFileKind(file: ChatFile): string {
	if (file.mime_type.startsWith("image/")) return "image";
	if (file.mime_type === "application/pdf") return "pdf";
	const extension = /\.([a-z0-9]{1,8})$/i.exec(file.file_name)?.[1];
	return extension ? extension.toLowerCase() : "file";
}

/** The files of a chat message on one line: name, id and kind. The name is the sender's text: it is data. */
export function chatFilesText(files: readonly ChatFile[] | undefined): string {
	if (!files?.length) return "";
	return files.map((file) => (file.deleted ? `${file.file_name} (deleted)` : `${file.file_name} (file_id ${file.id}, ${chatFileKind(file)})`)).join("; ");
}
