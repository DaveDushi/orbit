/**
 * The deployed version and what is new, for "which version are you?" and "what's new?". The revision comes from the
 * web build manifest that Orbit serves (`/orbit-build.json`); the notes come from `whats-new.md`, which each deploy
 * updates. The section changes only when Orbit is deployed, so the prompt cache stays.
 */

import { readFileSync } from "node:fs";

export type Release = { readonly date: string; readonly lines: readonly string[] };

/** The entries of `whats-new.md`, newest first: `## YYYY-MM-DD`, then `- ` lines. Other lines are comments. */
export function parseWhatsNew(text: string): Release[] {
	const releases: { date: string; lines: string[] }[] = [];
	for (const line of text.replace(/<!--[\s\S]*?-->/g, "").split("\n")) {
		const heading = /^## (\d{4}-\d{2}-\d{2})\s*$/.exec(line);
		if (heading) releases.push({ date: heading[1], lines: [] });
		else if (line.startsWith("- ") && releases.length) releases.at(-1)!.lines.push(line.slice(2).trim());
	}
	return releases;
}

const WHATS_NEW = parseWhatsNew(readFileSync(new URL("./whats-new.md", import.meta.url), "utf8"));
const SHOWN = 5;

/** The prompt section. `revision`: the build's git revision, `development`, or `undefined` when Orbit did not answer. */
export function versionBlock(revision: string | undefined, releases: readonly Release[] = WHATS_NEW): string {
	const version = !revision
		? "The version of this Orbit server is unknown right now."
		: revision === "development" || !/^[0-9a-f]{7,40}$/.test(revision)
			? `This Orbit server is a ${revision} build (not a release).`
			: `This Orbit server runs version ${revision.slice(0, 7)} (its git revision).`;
	const deployed = releases[0] ? ` The newest release below was deployed on ${releases[0].date}.` : "";
	const notes = releases.slice(0, SHOWN).map((release) => `${release.date}\n${release.lines.map((line) => `- ${line}`).join("\n")}`);
	return `Version and what's new (for "which version are you?" and "what's new?"):
${version}${deployed}
${notes.length ? `What's new in Orbit, newest first:\n${notes.join("\n")}` : "No release notes are available."}`;
}

const TTL_MS = 10 * 60_000;
let cached: { revision: string; at: number } | undefined;

/** Orbit's build revision, read again at most every 10 minutes, so a new deploy shows without a sidecar restart. */
export async function orbitRevision(orbit: { buildRevision(): Promise<string> }, now = Date.now()): Promise<string | undefined> {
	if (cached && now - cached.at < TTL_MS) return cached.revision;
	try {
		cached = { revision: await orbit.buildRevision(), at: now };
		return cached.revision;
	} catch {
		// Orbit is down or restarting: the last known revision, and a new try in the next run.
		return cached?.revision;
	}
}
