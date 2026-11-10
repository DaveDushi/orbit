/**
 * The Ask bar: a message sent from a page other than Chat carries that page and the item open there (Orbit's
 * `PageContext`). It comes from the driver's browser, so it is untrusted: the item's title is shown as data, and the
 * model reads the item only with its tools, as the driver.
 */

import { untrusted } from "./files.ts";

export type PageContext = {
	readonly page: string;
	readonly item?: { readonly kind: string; readonly id: string; readonly title: string } | null;
};

const PAGES: Record<string, string> = {
	home: "Home",
	tasks: "Tasks",
	docs: "Docs",
	drive: "Drive",
	activity: "Activity",
	settings: "Workspace settings",
	account: "Account settings",
	admin: "Admin",
	apps: "Apps",
	other: "Orbit",
};

/** What each kind of item is, and the tool that reads it. */
const ITEMS: Record<string, { what: string; read: string }> = {
	task: { what: "task", read: "get_task, task_id" },
	doc: { what: "Docs page", read: "read_page, page_id" },
	drive_folder: { what: "Drive folder", read: "drive_list, folder_id" },
};

const ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * The message text with the page it was sent from, or the text alone when there is no page context (Chat, an older
 * Orbit) or Orbit sent one that this version does not know.
 */
export function withPageContext(text: string, context: PageContext | null | undefined): string {
	const page = context ? PAGES[context.page] : undefined;
	if (!page) return text;
	const item = context?.item;
	const kind = item ? ITEMS[item.kind] : undefined;
	if (!item || !kind || !ID.test(item.id)) return `${text}\n\nSent from the Ask bar on the ${page} page.`;
	const data = { kind: item.kind, id: item.id, title: item.title.slice(0, 200) };
	return `${text}\n\nSent from the Ask bar on the ${page} page, with this ${kind.what} open (when the driver means "this", "here" or "it", read it with ${kind.read} = the id below):\n${untrusted("page context", data)}`;
}
