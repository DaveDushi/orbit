import assert from "node:assert/strict";
import { test } from "node:test";

import { mimeOf, safeName, workPath } from "../src/media.ts";
import { aboutOrbit } from "../src/prompt.ts";
import { Sandbox, workId } from "../src/sandbox.ts";
import { orbitExtension } from "../src/tools.ts";

test("paths stay inside /work", () => {
	assert.equal(workPath("clip.mp4"), "clip.mp4");
	assert.equal(workPath("/work/frames/001.png"), "frames/001.png");
	assert.equal(workPath("./a//b/./c.jpg"), "a/b/c.jpg");
	for (const outside of ["", "/work", "/work/", "/etc/passwd", "../x", "a/../../x", "frames/..", "/workx/a", " /root/.ssh/id"]) {
		assert.equal(workPath(outside), undefined, outside);
	}
});

test("names from Drive or the model become plain file names", () => {
	assert.equal(safeName("../../etc/passwd"), "passwd");
	assert.equal(safeName("folder\\photo.jpg"), "photo.jpg");
	assert.equal(safeName("-rf.mp4"), "rf.mp4");
	assert.equal(safeName(".hidden"), "hidden");
	assert.equal(safeName("a\nb\u0007.png"), "ab.png");
	assert.equal(safeName(".."), "file");
});

test("a saved file gets the MIME type of its extension", () => {
	assert.equal(mimeOf("clip (trimmed).MP4"), "video/mp4");
	assert.equal(mimeOf("page-1.png"), "image/png");
	assert.equal(mimeOf("notes"), "application/octet-stream");
});

test("every conversation gets its own short /work name", () => {
	assert.match(workId("conv-1"), /^[0-9a-f]{32}$/);
	assert.notEqual(workId("conv-1"), workId("conv-2"));
	assert.equal(workId("conv-1"), workId("conv-1"));
});

test("the media tools and their rules come only with a sandbox", () => {
	const deps = { orbit: undefined as never, publicUrl: "", maxToolCalls: 1, botUserId: () => undefined };
	const names = (extension: { tools?: readonly { name: string }[] }) => (extension.tools ?? []).map((tool) => tool.name);
	const without = names(orbitExtension(deps));
	const withSandbox = names(orbitExtension({ ...deps, sandbox: new Sandbox("/nonexistent.sock") }));
	for (const tool of ["media_shell", "work_from_drive", "view_image", "work_to_drive"]) {
		assert.ok(!without.includes(tool), tool);
		assert.ok(withSandbox.includes(tool), tool);
	}
	assert.doesNotMatch(aboutOrbit("Donna"), /media sandbox|cannot hear/);
	assert.match(aboutOrbit("Donna", true), /cannot hear audio/);
});
