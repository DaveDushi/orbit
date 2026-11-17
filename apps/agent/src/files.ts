import type { Context } from "@earendil-works/chord/context";
import { Type } from "@earendil-works/pi-ai";
import type { ToolExecutionApi, ToolExecutionResult } from "@earendil-works/pi-durable";

import { ThreadDoc } from "./docs.ts";

/**
 * Page and chat files for the model: a read of one part of a file becomes text that fits in one tool result
 * (`untrusted` cuts at 12,000 characters), and says where the next part starts.
 */

/** Text from Orbit content is data for the model, never instructions. The content cannot close its own block. */
export function untrusted(label: string, value: unknown): string {
	const text = typeof value === "string" ? value : JSON.stringify(value, null, 1);
	return dataBlock(label, text.length > 12_000 ? `${text.slice(0, 12_000)}\n… (cut)` : text);
}

/** `untrusted` without the cut: for text that has its own bound (a script's output). */
export function dataBlock(label: string, text: string): string {
	return `<orbit-data source="${label}">\n${text.replace(/<\/orbit-data/gi, "<\\/orbit-data")}\n</orbit-data>`;
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

/**
 * Whole-file reads (`all: true` on read_file, read_chat_file and drive_read): only a script may make them, so the rows
 * stay in the sandbox and only the script's short output reaches the model. The tool reads every part of the file
 * through the same routes as a one-part read (the driver's permissions), in one tool call. All of a run's whole-file
 * reads together parse at most `MAX_RUN_READ_BYTES`.
 */
export const MAX_RUN_READ_BYTES = 32 * 1024 * 1024;
/**
 * The file readers' result limit (Pi's default cuts at 50 KB): a whole file is one JSON line for the script, and
 * cut JSON cannot be parsed. Quotes and commas make it larger than the parsed content.
 */
export const WHOLE_OUTPUT_LIMITS = { maxBytes: 2 * MAX_RUN_READ_BYTES, maxLines: 1_000_000 };
/** The most per request: Orbit cuts it to its own most rows (1,000) or characters. */
export const WHOLE_PART = 200_000;

/** The `all` argument of the file readers. */
export const WHOLE_FILE = Type.Optional(
	Type.Boolean({ description: "Only in a codemode script: the whole file in one call, as JSON {rows (header row first) or text, sheets, sheet}. Keep the rows in the script; output only a short result." }),
);

/** Nested calls (from a script) with `all: true`, by call id: the tools hook adds them, the read takes its own. */
export const wholeReadCalls = new Set<string>();

/** The whole file as a script gets it: `rows` (the header row first) or `text`. */
export type WholeFile = {
	file_id: string;
	name: string;
	kind: string;
	sheets: string[];
	sheet: string | null;
	rows: string[][] | null;
	text: string | null;
	no_text: boolean;
};

export class ReadCapError extends Error {}

const mb = (bytes: number): string => `${(bytes / (1024 * 1024)).toFixed(1)} MB`;

/**
 * Reads every part of a file with `part(offset, limit)`. `read.bytes` counts the parsed content (cells and text) as it
 * comes, also when the read stops; past `budget` it stops with a `ReadCapError`.
 */
export async function readWhole(part: (offset: number, limit: number) => Promise<FileContent>, budget: number, read: { bytes: number }): Promise<WholeFile> {
	let whole: WholeFile | undefined;
	let offset = 0;
	for (;;) {
		const content = await part(offset, WHOLE_PART);
		whole ??= { file_id: content.file.id, name: content.file.file_name, kind: content.file.kind, sheets: content.sheets, sheet: content.sheet, rows: content.rows ? [] : null, text: content.rows ? null : "", no_text: content.no_text };
		read.bytes += content.rows ? content.rows.reduce((size, row) => row.reduce((cells, cell) => cells + cell.length + 1, size), 0) : (content.text?.length ?? 0);
		if (read.bytes > budget) {
			throw new ReadCapError(
				`Stopped: whole-file reads may parse at most ${mb(MAX_RUN_READ_BYTES)} per run, and this one went over it (${mb(read.bytes)} read so far${budget < MAX_RUN_READ_BYTES ? ` with ${mb(MAX_RUN_READ_BYTES - budget)} read before in this run` : ""}). Tell the driver the file is too large to read at once; read one part with offset, or ask for a smaller file or one sheet.`,
			);
		}
		if (content.rows) whole.rows?.push(...content.rows);
		else whole.text += content.text ?? "";
		// An offset that does not move on would never end.
		if (content.next_offset === null || content.next_offset <= offset) return whole;
		offset = content.next_offset;
	}
}

/**
 * A tool's `all: true` read: only from a script, within the run's budget (`ThreadDoc.readBytes`). The script gets the
 * whole file as JSON text.
 */
export async function wholeFileResult(api: ToolExecutionApi, context: Context, part: (offset: number, limit: number) => Promise<FileContent>): Promise<ToolExecutionResult> {
	if (!wholeReadCalls.delete(api.callId)) {
		return { output: [{ type: "text", text: "all: true works only in a codemode script: the script reads the whole file and returns a short result. Here, read one part (offset)." }], isError: true };
	}
	const used = (await api.snapshot(ThreadDoc, api.conversationId, context))?.readBytes ?? 0;
	const read = { bytes: 0 };
	try {
		const json = JSON.stringify(await readWhole(part, MAX_RUN_READ_BYTES - used, read));
		if (Buffer.byteLength(json) > WHOLE_OUTPUT_LIMITS.maxBytes) throw new ReadCapError(`Stopped: this file is ${mb(Buffer.byteLength(json))} as JSON, more than a whole-file read can return (${mb(WHOLE_OUTPUT_LIMITS.maxBytes)}). Tell the driver the file is too large to read at once; read one part with offset, or ask for a smaller file or one sheet.`);
		return { output: [{ type: "text", text: json }] };
	} catch (error) {
		if (error instanceof ReadCapError) return { output: [{ type: "text", text: error.message }], isError: true };
		throw error;
	} finally {
		await api.commit(async (tx) => {
			const doc = await tx.doc(ThreadDoc, api.conversationId);
			doc.readBytes = (doc.readBytes ?? 0) + read.bytes;
		}, context);
	}
}

/** A file of a chat message, as Orbit lists it on the message. */
export type ChatFile = { id: string; file_name: string; mime_type: string; size_bytes?: number; deleted?: boolean };

/**
 * The files of the message that starts or joins a run, after its text, so the model knows them without a call.
 * `canView`: the run has view_image, so the model looks at the images itself.
 */
export function withMessageFiles(text: string, files: readonly ChatFile[] | undefined, canView = false): string {
	const list = chatFilesText(files);
	if (!list) return text;
	const images = canView && files?.some((file) => !file.deleted && chatFileKind(file) === "image");
	const how = images ? "look at the images with view_image (chat_file_ids) before you answer; read the other files with read_chat_file" : "read them with read_chat_file";
	return `${text}\n\nFiles sent with this message (${how}):\n${untrusted("message files", list)}`;
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
