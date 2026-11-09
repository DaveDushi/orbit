/**
 * Escape tests of the media sandbox, against a real broker. They run only with AGENT_SANDBOX_TEST_SOCKET, the socket
 * of a test copy (`deploy/donna-sandbox/setup.sh --name donna-test --user donna-sbx-test --client-group root
 * --agent-unit none --no-apt --quota-mb 256`), and AGENT_SANDBOX_TEST_USER, its sandbox user.
 */

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { test } from "node:test";

import { Sandbox, workId } from "../src/sandbox.ts";

const socket = process.env.AGENT_SANDBOX_TEST_SOCKET;
const user = process.env.AGENT_SANDBOX_TEST_USER ?? "donna-sbx-test";
const skip = socket ? false : "set AGENT_SANDBOX_TEST_SOCKET to the socket of a test sandbox";
const sandbox = new Sandbox(socket ?? "");
const work = workId(randomUUID());
const text = (buffer: Buffer) => buffer.toString("utf8");
/** The sandbox user's processes on the host (`ps` fails when there are none). */
function processesOfUser(): string {
	try {
		return execFileSync("ps", ["-u", user, "-o", "pid=,comm="], { encoding: "utf8" }).trim();
	} catch {
		return "";
	}
}

test("the sandbox runs as its own user, with media tools and an empty environment", { skip }, async () => {
	process.env.ORBIT_AGENT_SECRET_CANARY = "canary-7d1f3e";
	const result = await sandbox.run(work, 'id -un; echo "$HOME $PWD"; env; cat /proc/1/environ | wc -c; ffmpeg -version | head -1; convert -version | head -1');
	const out = text(result.stdout);
	assert.equal(result.exit, 0, text(result.stderr));
	assert.match(out, /^donna\n\/work \/work\n/);
	assert.doesNotMatch(out, /canary|ORBIT|AGENT|SECRET|TOKEN/i);
	assert.match(out, /\n0\n/, "bwrap's first process has an empty environment");
	assert.match(out, /ffmpeg version/);
	assert.match(out, /ImageMagick/);
});

test("there is no network", { skip }, async () => {
	const result = await sandbox.run(
		work,
		"(exec 3<>/dev/tcp/1.1.1.1/443) 2>/dev/null || echo refused; (exec 3<>/dev/tcp/127.0.0.1/8000) 2>/dev/null || echo refused; cat /proc/net/dev | tail -n +3 | cut -d: -f1",
	);
	const out = text(result.stdout);
	assert.equal(out.match(/refused/g)?.length, 2, out);
	assert.doesNotMatch(out, /eth|ens|enp|docker|wl/);
});

test("it sees no secrets, homes, Orbit data, agent files, sockets or other processes", { skip }, async () => {
	const paths = ["/root", "/home", "/opt", "/opt/orbit-agent", "/var/lib/orbit", "/var/lib/orbit-agent", "/etc/orbit", "/var/backups", "/run", "/usr/local/bin/orbit", "/etc/shadow"];
	const result = await sandbox.run(work, 'for p in "$@"; do if [ -e "$p" ]; then echo "SEEN $p"; fi; done; ls /proc | grep -cE "^[0-9]+$"; ps -eo comm --no-headers 2>/dev/null', { args: paths });
	const out = text(result.stdout);
	assert.doesNotMatch(out, /SEEN/, out);
	const lines = out.trim().split("\n");
	assert.ok(Number(lines[0]) <= 5, `only the sandbox's own processes: ${out}`);
	assert.doesNotMatch(out, /node|orbit|systemd|python/);
});

test("the system is read-only; only /work and /tmp take writes", { skip }, async () => {
	const result = await sandbox.run(work, "for p in /usr/x /etc/x /x /usr/local/x; do touch $p 2>/dev/null && echo WROTE $p; done; touch /work/ok /tmp/ok && echo fine");
	assert.equal(text(result.stdout).trim(), "fine");
});

test("it cannot make new namespaces", { skip }, async () => {
	const result = await sandbox.run(work, "unshare --user --map-root-user true 2>/dev/null && echo NEW-USERNS; unshare --net true 2>/dev/null && echo NEW-NETNS; echo done");
	assert.equal(result.stdout.toString(), "done\n");
});

test("one conversation cannot see another's /work", { skip }, async () => {
	await sandbox.run(work, "echo private > mine.txt");
	const other = await sandbox.run(workId(randomUUID()), "ls -A /work; cat /work/mine.txt 2>/dev/null || true");
	assert.doesNotMatch(text(other.stdout), /private|mine/);
	const same = await sandbox.run(work, "cat mine.txt");
	assert.equal(text(same.stdout), "private\n");
});

test("a disk fill stops at the /work quota", { skip }, async () => {
	const result = await sandbox.run(work, "dd if=/dev/zero of=fill bs=1M count=2000 status=none; echo dd=$?; du -m fill | cut -f1; df -m --output=size /work | tail -1; rm -f fill", { timeoutSeconds: 120 });
	const [status, used, size] = text(result.stdout).trim().split("\n");
	assert.notEqual(status, "dd=0", text(result.stderr));
	assert.match(text(result.stderr), /No space left|File too large/);
	assert.ok(Number(used) <= Number(size) && Number(size) <= 2048, `${used} MiB used of ${size}`);
});

test("the time limit kills the run and everything it started", { skip }, async () => {
	const started = Date.now();
	const result = await sandbox.run(work, "(sleep 300 &); setsid sleep 301 & sleep 302", { timeoutSeconds: 2 });
	assert.equal(result.timedOut, true);
	assert.ok(Date.now() - started < 15_000);
	await new Promise((resolve) => setTimeout(resolve, 1000));
	const left = processesOfUser();
	assert.equal(left, "", `processes of ${user} left: ${left}`);
});

test("a fork bomb is contained, and the next run works", { skip }, async () => {
	const bomb = await sandbox.run(work, "bomb() { bomb | bomb & }; bomb; sleep 3; echo still-here", { timeoutSeconds: 8 });
	assert.ok(bomb.timedOut || bomb.exit !== null);
	const next = await sandbox.run(work, "echo ok", { timeoutSeconds: 10 });
	assert.equal(text(next.stdout), "ok\n");
	await new Promise((resolve) => setTimeout(resolve, 1000));
	assert.equal(processesOfUser(), "");
});

test("memory is capped", { skip }, async () => {
	const result = await sandbox.run(work, "python3 -c 'b = bytearray(3 * 1024**3); print(len(b))'", { timeoutSeconds: 60 });
	assert.notEqual(result.exit, 0);
	assert.doesNotMatch(text(result.stdout), /3221225472/);
});

test("files go in through stdin and come out through stdout, byte for byte", { skip }, async () => {
	const bytes = Buffer.from(Array.from({ length: 300_000 }, (_, index) => (index * 7) % 256));
	const put = await sandbox.run(work, 'cat > "$1"', { args: ["blob.bin"], stdin: bytes });
	assert.equal(put.exit, 0, text(put.stderr));
	const get = await sandbox.run(work, 'cat -- "$1"', { args: ["blob.bin"], stdoutMax: 1_000_000 });
	assert.ok(get.stdout.equals(bytes));
	const cut = await sandbox.run(work, 'cat -- "$1"', { args: ["blob.bin"], stdoutMax: 1000 });
	assert.equal(cut.stdout.length, 1000);
	assert.equal(cut.stdoutTruncated, true);
});
