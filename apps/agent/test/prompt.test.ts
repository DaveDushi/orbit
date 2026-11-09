// About Orbit names every page of the web app's sidebar and account menu, read from their sources; and the version
// section is built from the build revision and `whats-new.md`.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { aboutOrbit } from "../src/prompt.ts";
import { orbitRevision, parseWhatsNew, versionBlock } from "../src/version.ts";

const web = (path: string) => readFileSync(new URL(`../../web/src/${path}`, import.meta.url), "utf8");
const all = (text: string, pattern: RegExp) => [...text.matchAll(pattern)].map((match) => match[1].trim());

/** The labels of the pages people open from the sidebar and the account menu. */
function pageLabels(): Map<string, string[]> {
	const sidebar = web("app/shell/SidebarNav.tsx");
	// The settings areas only: the Agentic page's tabs below them are not sidebar pages.
	const settings = web("features/settings/settingsNav.ts").split("\nconst under")[0];
	const menu = web("app/shell/UserMenu.tsx");
	return new Map([
		// WORKSPACE_LINKS and TASK_PRESETS, then the fixed <SidebarNavItem label="…"> rows (Views, Trash, Settings, Admin…).
		["SidebarNav.tsx", [...all(sidebar, /\{ (?:to|view): [^,]+, label: '([^']+)'/g), ...all(sidebar, /<SidebarNavItem [^>]*\slabel="([^"]+)"/g)]],
		["settingsNav.ts", [...all(settings, /\{ to: '[^']+', label: '([^']+)'/g), ...all(settings, /\btitle: '([^']+)'/g)]],
		["UserMenu.tsx", all(menu, /<DropdownMenuItem\b.*>\n\s*<\w+ \/>\n\s*([^<{}\n]+)\n/g)],
	]);
}

test("About Orbit names every page of the sidebar, the settings areas and the account menu", () => {
	const about = aboutOrbit("Donna").toLowerCase();
	const labels = pageLabels();
	// The patterns still find the pages: a changed source must not make this test pass with nothing to check.
	assert.ok(labels.get("SidebarNav.tsx")!.length >= 15, `sidebar labels: ${labels.get("SidebarNav.tsx")}`);
	assert.ok(labels.get("settingsNav.ts")!.length >= 30, `settings labels: ${labels.get("settingsNav.ts")}`);
	assert.deepEqual(labels.get("UserMenu.tsx"), ["Account settings", "Appearance", "Suggest a change"]);
	for (const known of ["Home", "Drive", "Due soon", "Views", "All projects", "Admin"]) assert.ok(labels.get("SidebarNav.tsx")!.includes(known), known);
	for (const known of ["API tokens", "Keyboard shortcuts", "Audit log"]) assert.ok(labels.get("settingsNav.ts")!.includes(known), known);

	const missing = [...labels].flatMap(([source, names]) => names.filter((name) => !about.includes(name.toLowerCase())).map((name) => `${name} (${source})`));
	assert.deepEqual(missing, [], "add these pages to aboutOrbit() in apps/agent/src/prompt.ts");
});

test("whats-new.md: dated entries, newest first, each with lines", () => {
	const releases = parseWhatsNew(readFileSync(new URL("../src/whats-new.md", import.meta.url), "utf8"));
	assert.ok(releases.length > 0 && releases.length <= 8, "keep 1 to 8 entries");
	for (const release of releases) {
		assert.ok(!Number.isNaN(Date.parse(release.date)), `${release.date} is a date`);
		assert.ok(release.lines.length > 0, `${release.date} has lines`);
	}
	const dates = releases.map((release) => release.date);
	assert.deepEqual(dates, [...dates].sort().reverse(), "newest first");
	assert.equal(new Set(dates).size, dates.length, "one entry per date");
});

test("parseWhatsNew skips the title and comments and keeps lines under their date", () => {
	const text = "# What's new\n<!--\n## 2000-01-01\n- not an entry\n-->\n\n## 2026-11-09\n- One.\n- Two.\n\n## 2026-11-07\n- Three.\n";
	assert.deepEqual(parseWhatsNew(text), [
		{ date: "2026-11-09", lines: ["One.", "Two."] },
		{ date: "2026-11-07", lines: ["Three."] },
	]);
});

test("the version section names the short revision, the deploy date and the newest five releases", () => {
	const releases = Array.from({ length: 6 }, (_, index) => ({ date: `2026-11-0${9 - index}`, lines: [`Change ${index}.`] }));
	const block = versionBlock("4eb9a16edf3598b5f8ec9c0751cc25525321bd8b", releases);
	assert.match(block, /runs version 4eb9a16 \(its git revision\)\. The newest release below was deployed on 2026-11-09\./);
	assert.match(block, /2026-11-09\n- Change 0\.\n2026-11-08\n- Change 1\./);
	assert.ok(block.includes("Change 4.") && !block.includes("Change 5."), "only the newest five");

	assert.match(versionBlock("development", releases), /a development build \(not a release\)/);
	assert.match(versionBlock(undefined, []), /version of this Orbit server is unknown[\s\S]*No release notes/);
});

test("orbitRevision reads Orbit again after 10 minutes and keeps the last revision while Orbit fails", async () => {
	let answer: string | Error = "aaaaaaa";
	let reads = 0;
	const orbit = {
		async buildRevision() {
			reads++;
			if (answer instanceof Error) throw answer;
			return answer;
		},
	};
	assert.equal(await orbitRevision(orbit, 0), "aaaaaaa");
	answer = "bbbbbbb";
	assert.equal(await orbitRevision(orbit, 9 * 60_000), "aaaaaaa", "cached");
	assert.equal(reads, 1);
	assert.equal(await orbitRevision(orbit, 10 * 60_000), "bbbbbbb", "a new deploy shows");
	answer = new Error("down");
	assert.equal(await orbitRevision(orbit, 21 * 60_000), "bbbbbbb", "the last known revision");
	assert.equal(await orbitRevision(orbit, 21 * 60_000 + 1), "bbbbbbb");
	assert.equal(reads, 4, "a failed read is tried again in the next run");
});
