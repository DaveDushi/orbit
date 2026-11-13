import assert from "node:assert/strict";
import { test } from "node:test";

import { mimeOf, safeName, workPath } from "../src/media.ts";
import { aboutOrbit } from "../src/prompt.ts";
import { Sandbox, workId } from "../src/sandbox.ts";
import { OrbitError, type OrbitClient } from "../src/orbit.ts";
import { MEDIA_TOOLS, orbitExtension } from "../src/tools.ts";

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

/** view_image against a fake Orbit and a fake sandbox: what Orbit was asked as whom, and what reached /work. */
function viewHarness(download: (path: string) => Promise<Response>) {
	const downloads: { userId: string; workspaceId: string; path: string }[] = [];
	const written: { path: string; bytes: Buffer }[] = [];
	const orbit = {
		asUser: async () => undefined,
		downloadAsUser: async (userId: string, workspaceId: string, path: string) => {
			downloads.push({ userId, workspaceId, path });
			return download(path);
		},
	} as unknown as OrbitClient;
	const ran = (stdout: string, stderr = "") => ({ exit: 0, signal: null, timedOut: false, stdout: Buffer.from(stdout), stderr: Buffer.from(stderr), stdoutTruncated: false, stderrTruncated: false });
	const sandbox = {
		run: async (_work: string, script: string, options: { args?: string[]; stdin?: Buffer }) => {
			if (script.startsWith("mkdir -p .view")) {
				written.push({ path: options.args![0], bytes: options.stdin! });
				return ran("");
			}
			// The ImageMagick step: the size on stderr, the downscaled JPEG on stdout.
			return ran("jpeg-bytes", "1179 2556 PNG\n");
		},
	} as unknown as Sandbox;
	orbitExtension({ orbit, publicUrl: "https://orbit.test", maxToolCalls: 10, botUserId: () => undefined, sandbox });
	const api = {
		conversationId: "c1",
		callId: "call-1",
		snapshot: async () => ({ driverId: "ann", driverName: "Ann", workspaceId: "w1", conversationId: "c1", toolCalls: 0 }),
		commit: async () => undefined,
	};
	const tool = MEDIA_TOOLS.find((candidate) => candidate.name === "view_image") as unknown as {
		execute(args: unknown, api: unknown, context: unknown): Promise<{ output: { type: string; text?: string; data?: string; mimeType?: string }[]; isError?: boolean }>;
	};
	return { downloads, written, view: (args: unknown) => tool.execute(args, api, {}) };
}

test("view_image shows an image sent in chat, fetched as the driver, and explains the files Orbit refuses", async () => {
	const png = Buffer.from("png-bytes");
	const { downloads, written, view } = viewHarness(async (path) => {
		if (path.startsWith("/chat/files/f-img")) return new Response(png, { headers: { "content-length": String(png.length) } });
		// Orbit: not an image, or a file the driver cannot open.
		throw new OrbitError(path.startsWith("/chat/files/f-text") ? 403 : 404, { code: "chat_forbidden" }, "GET");
	});
	const result = await view({ chat_file_ids: ["f-img", "f-text", "f-secret"] });
	assert.equal(result.isError, false);
	assert.deepEqual(
		downloads.map((call) => [call.userId, call.workspaceId, call.path]),
		[
			["ann", "w1", "/chat/files/f-img?inline=true"],
			["ann", "w1", "/chat/files/f-text?inline=true"],
			["ann", "w1", "/chat/files/f-secret?inline=true"],
		],
	);
	assert.deepEqual(written, [{ path: ".view/chat-f-img", bytes: png }]);
	const image = result.output.find((part) => part.type === "image")!;
	assert.equal(image.mimeType, "image/jpeg");
	assert.equal(Buffer.from(image.data!, "base64").toString(), "jpeg-bytes");
	const texts = result.output.filter((part) => part.type === "text").map((part) => part.text!);
	assert.match(texts[0], /"width": 1179/);
	assert.match(texts[1], /f-text: the driver cannot open it, or it is not an image \(Orbit answered 403\)/);
	assert.match(texts[2], /f-secret: .*404/);
	// No base64 in the text parts: the image goes to the model only as image input.
	assert.ok(texts.every((text) => !text.includes(image.data!)));
});

test("view_image refuses a chat image over the size limit before it reads the bytes", async () => {
	let read = false;
	// No read ahead: `pull` runs only when someone reads the body.
	const body = new ReadableStream(
		{
			pull() {
				read = true;
			},
		},
		{ highWaterMark: 0 },
	);
	const { written, view } = viewHarness(async () => new Response(body, { headers: { "content-length": String(65 * 1024 * 1024) } }));
	const result = await view({ chat_file_ids: ["f-huge"] });
	assert.equal(result.isError, true);
	assert.match(result.output[0].text!, /65 MiB; the limit here is 64 MiB/);
	assert.deepEqual(written, []);
	assert.equal(read, false);
});
