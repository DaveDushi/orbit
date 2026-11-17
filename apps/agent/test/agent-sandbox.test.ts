/**
 * Escape, limit and lifecycle tests of the agent sandbox broker (deploy/agent-sandbox/broker.py: one gVisor sandbox
 * per chat thread), against a real broker. Run them as root, only on a TEST COPY (never production):
 *
 *   deploy/agent-sandbox/setup.sh --name orbit-sbx-test --slice-prefix orbit-sbx-test --net 10.202 --net-prefix sbt \
 *     --proxy-port 13128 --quota-mb 256 --min-free-mb 4096 --client-group root --image <agent image> \
 *     --key /etc/orbit-sbx-test/broker.key --orbit-url http://127.0.0.1:13129
 *   AGENT_SBX_TEST_NAME=orbit-sbx-test node --test test/agent-sandbox.test.ts
 *
 * The tests stand in for Orbit: they sign tickets with the test key and answer the broker's call-end reports.
 */

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHmac, randomBytes, randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
import { createServer, type Server as HttpServer } from "node:http";
import { createConnection, createServer as createTcpServer } from "node:net";
import { after, before, test } from "node:test";

const name = process.env.AGENT_SBX_TEST_NAME;
const skip = name ? false : "set AGENT_SBX_TEST_NAME to the name of a test copy of the agent sandbox broker";
const run = `/run/${name}`;
const socketPath = `${run}/sandbox.sock`;
const env = name ? Object.fromEntries(readFileSync(`/etc/${name}/broker.env`, "utf8").trim().split("\n").map((line) => line.split("=", 2))) : {};
const net: string = env.SBX_NET ?? "";
const netPrefix: string = env.SBX_NET_PREFIX ?? "";
const proxyPort = Number(env.SBX_PROXY_PORT);
const orbitPort = Number(new URL(env.SBX_ORBIT_URL || "http://127.0.0.1:0").port);
const quotaMb = Number(env.SBX_QUOTA_MB);
const key = name ? readFileSync(env.SBX_KEY_FILE).toString("utf8").trim() : "";
const derived = (label: string) => createHmac("sha256", key).update(label).digest();

const AGENT = "0190a0b0-0000-7000-8000-00000000000a";
const OTHER_AGENT = "0190a0b0-0000-7000-8000-00000000000b";
const newWork = () => randomBytes(16).toString("hex");

type Limits = { timeout_s?: number; memory_mb?: number; cpu?: number; idle_minutes?: number; disk_mb?: number };
type Claims = { call: string; agent: string; work: string; exp: number; limits: Limits; token: string; proxy_password: string };
type Result = { exit: number | null; timed_out: boolean; interrupted: string | null; error?: string; egress_pending?: unknown[]; cpu_ms?: number; stdout: string; stderr: string };

function seal(secrets: object): string {
	const plain = Buffer.from(JSON.stringify(secrets));
	const nonce = randomBytes(16);
	const blocks = [];
	for (let i = 0; i * 32 < plain.length; i++) {
		const counter = Buffer.alloc(4);
		counter.writeUInt32BE(i);
		blocks.push(createHmac("sha256", derived("orbit-sandbox/seal")).update(Buffer.concat([nonce, counter])).digest());
	}
	const stream = Buffer.concat(blocks);
	return Buffer.concat([nonce, plain.map((byte, i) => byte ^ stream[i])]).toString("base64url");
}

function ticket(claims: Claims, macKey = derived("orbit-sandbox/ticket")): string {
	const { token, proxy_password, ...open } = claims;
	const payload = Buffer.from(JSON.stringify({ ...open, sealed: seal({ token, proxy_password }) })).toString("base64url");
	return `v1.${payload}.${createHmac("sha256", macKey).update(`v1.${payload}`).digest("base64url")}`;
}

function claimsFor(work: string, options: { agent?: string; limits?: Limits; exp?: number } = {}): Claims {
	return {
		call: randomUUID(),
		agent: options.agent ?? AGENT,
		work,
		exp: options.exp ?? Date.now() + 300_000,
		limits: { timeout_s: 60, memory_mb: 1024, cpu: 1, idle_minutes: 30, ...options.limits },
		token: `orbit-agent.${randomBytes(12).toString("base64url")}`,
		proxy_password: randomBytes(12).toString("base64url"),
	};
}

function request(body: object): Promise<Result> {
	return new Promise((resolve, reject) => {
		const socket = createConnection({ path: socketPath });
		const out: Buffer[] = [];
		const err: Buffer[] = [];
		let end: Record<string, unknown> | undefined;
		let buffered = Buffer.alloc(0);
		socket.on("connect", () => socket.write(`${JSON.stringify(body)}\n`));
		socket.on("data", (chunk: Buffer) => {
			buffered = Buffer.concat([buffered, chunk]);
			while (buffered.length >= 5 && buffered.length >= 5 + buffered.readUInt32BE(1)) {
				const length = buffered.readUInt32BE(1);
				const kind = String.fromCharCode(buffered[0]);
				const payload = buffered.subarray(5, 5 + length);
				if (kind === "o") out.push(payload);
				else if (kind === "e") err.push(payload);
				else end = JSON.parse(payload.toString("utf8"));
				buffered = buffered.subarray(5 + length);
			}
		});
		socket.on("error", reject);
		socket.on("close", () =>
			end
				? resolve({ ...(end as Result), stdout: Buffer.concat(out).toString("utf8"), stderr: Buffer.concat(err).toString("utf8") })
				: reject(new Error("the broker ended without an answer")),
		);
	});
}

/** One call in the thread sandbox `work`. */
function call(work: string, script: string, options: { agent?: string; limits?: Limits; claims?: Claims } = {}): Promise<Result> {
	return request({ op: "run", ticket: ticket(options.claims ?? claimsFor(work, options)), script });
}

const sh = (command: string, args: string[]) => execFileSync(command, args, { encoding: "utf8" });
/** pgrep and friends exit 1 when nothing matches: that is an empty answer, not an error. */
function count(command: string, args: string[], list = false): string {
	try {
		return sh(command, args).trim();
	} catch (error) {
		if ((error as { status?: number }).status === 1) return list ? "" : "0";
		throw error;
	}
}
type BrokerState = { sandboxes: Record<string, { agent: string; started: number; last_call: number; cgroup: string; netidx: number }> };
const state = (): BrokerState => JSON.parse(readFileSync(`${run}/state.json`, "utf8"));
/** Changes the broker's state under its lock (Python: Node has no flock). `s` is the state. */
function editState(code: string): void {
	sh("python3", ["-I", "-c", `import fcntl,json,sys,time\nl=open(sys.argv[1]+'/state.lock','w'); fcntl.flock(l, fcntl.LOCK_EX)\ns=json.load(open(sys.argv[1]+'/state.json'))\n${code}\njson.dump(s, open(sys.argv[1]+'/state.json','w'))`, run]);
}
const live = () => Object.keys(state().sandboxes);
const scopes = () => sh("systemctl", ["list-units", "--plain", "--no-legend", `${name}-*.scope`]).trim();
const netns = () => sh("ip", ["netns", "list"]).split("\n").filter((line) => line.startsWith(`${netPrefix}-`));
const containers = () => sh(env.SBX_RUNSC || `/usr/local/lib/${name}/gvisor/runsc`, [`--root=${run}/runsc`, "list", "-quiet"]).trim();
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

// Stands in for Orbit's call-end route: records the reports and answers one waiting egress request.
const ends: { path: string; auth: string; body: Record<string, unknown> }[] = [];
let orbit: HttpServer | undefined;
before(async () => {
	if (skip) return;
	await request({ op: "stop_all" });
	orbit = createServer((req, res) => {
		let body = "";
		req.on("data", (chunk) => (body += chunk));
		req.on("end", () => {
			ends.push({ path: req.url ?? "", auth: req.headers.authorization ?? "", body: JSON.parse(body || "{}") });
			res.setHeader("content-type", "application/json");
			res.end(JSON.stringify({ egress_pending: [{ host: "api.example.com", port: 443 }] }));
		});
	});
	await new Promise<void>((resolve) => orbit?.listen(orbitPort, "127.0.0.1", resolve));
});
after(async () => {
	if (skip) return;
	await request({ op: "stop_all" });
	orbit?.close();
});

test("a call runs as the sandbox user, in gVisor, with its limits and only its own environment", { skip }, async () => {
	process.env.ORBIT_AGENT_SECRET_CANARY = "canary-5c2e";
	const claims = claimsFor(newWork());
	const result = await call(claims.work, 'id -un; uname -r; ulimit -u; head -1 /proc/meminfo; echo "#env"; env | sort; echo "#pid1"; tr "\\0" "\\n" </proc/1/environ; echo "#ps"; ps -eo pid=,comm=', { claims });
	assert.equal(result.exit, 0, result.stderr);
	const [head, callEnv, pid1Env, ps] = result.stdout.split(/^#\w+\n/m);
	assert.deepEqual(head.trim().split("\n").slice(0, 3), ["agent", "4.19.0-gvisor", "128"]);
	assert.match(head, /MemTotal:\s+1048576 kB/);
	const names = (text: string) => [...new Set(text.match(/^[A-Za-z_]+(?==)/gm) ?? [])].sort();
	assert.deepEqual(names(callEnv), ["HOME", "HTTPS_PROXY", "LANG", "ORBIT_TOKEN", "ORBIT_URL", "PATH", "PWD", "PYTHONUSERBASE", "SHLVL", "TMPDIR", "_", "npm_config_prefix"]);
	assert.ok(callEnv.includes(`ORBIT_TOKEN=${claims.token}\n`));
	assert.ok(callEnv.includes(`HTTPS_PROXY=http://call:${claims.proxy_password}@`));
	assert.deepEqual(names(pid1Env), ["HOME", "LANG", "PATH", "PYTHONUSERBASE", "TMPDIR", "npm_config_prefix"], "PID 1 has no call secrets");
	assert.doesNotMatch(result.stdout, /canary|SECRET|APP_KEY|SBX_/);
	assert.match(ps, /^\s*1 tini$/m);
});

test("every gVisor process is in the thread's scope in the agent slice (no cgroup at the root, gVisor #10264)", { skip }, async () => {
	const work = newWork();
	await call(work, "true");
	const { cgroup } = state().sandboxes[work];
	assert.match(cgroup, new RegExp(`/${env.SBX_SLICE_PREFIX}-${AGENT.replaceAll("-", "")}\\.slice/${name}-${work}\\.scope$`));
	const placed = readFileSync(`/sys/fs/cgroup${cgroup}/cgroup.procs`, "utf8").trim().split("\n");
	const gvisor = count("pgrep", ["-f", `runsc-(sandbox|gofer).*${work}`], true).split("\n");
	assert.equal(gvisor.length, 2, "sentry and gofer");
	for (const pid of gvisor) assert.ok(placed.includes(pid), `${pid} is outside ${cgroup}`);
});

test("it sees no host files, keys or sockets; the system is read-only; privileged actions fail", { skip }, async () => {
	const paths = ["/etc/orbit", `/etc/${name}`, "/var/lib/orbit", "/var/lib/orbit-agent", `/var/lib/${name}`, "/root/.claude", "/home/exedev", "/run/docker.sock", "/opt/orbit-spike", "/opt/orbit-agent", `/run/${name}`];
	const result = await call(
		newWork(),
		`for p in ${paths.join(" ")}; do [ -e "$p" ] && echo "SEEN $p"; done
		 for p in /usr/x /etc/x /x /bin/x; do touch $p 2>/dev/null && echo "WROTE $p"; done
		 touch /work/ok /tmp/ok && echo writes-ok
		 cat /etc/shadow >/dev/null 2>&1 && echo SHADOW
		 mount -t tmpfs x /tmp 2>/dev/null && echo MOUNTED
		 echo 1 > /proc/sys/kernel/sysrq 2>/dev/null && echo SYSCTL
		 python3 -c 'import ctypes; l=ctypes.CDLL(None); print("PTRACE-1" if l.ptrace(16, 1, 0, 0) == 0 else "ptrace-refused")'
		 python3 -c 'import socket; socket.socket(socket.AF_PACKET, socket.SOCK_RAW)' 2>/dev/null && echo RAW
		 find / -xdev -perm -4000 -type f 2>/dev/null | head -3`,
	);
	assert.equal(result.stdout.trim(), "writes-ok\nptrace-refused", result.stdout + result.stderr);
});

test("egress: only TCP to the proxy port of its own /30; no DNS, no other host, no IPv6", { skip }, async () => {
	const work = newWork();
	await call(work, "true");
	const idx = state().sandboxes[work].netidx;
	const hostSide = `${net}.${idx}.1`;
	// Another live sandbox, so its /30 exists.
	const other = newWork();
	await call(other, "true");
	const otherHost = `${net}.${state().sandboxes[other].netidx}.1`;
	const seen: string[] = [];
	const proxy = createTcpServer((socket) => socket.once("data", (data) => (seen.push(data.toString("utf8").split("\r\n")[0]), socket.end("HTTP/1.1 403 Forbidden\r\n\r\n"))));
	await new Promise<void>((resolve) => proxy.listen(proxyPort, hostSide, resolve));
	try {
		const probe = `
import socket
def tcp(host, port):
    try:
        s = socket.create_connection((host, port), timeout=2); s.close(); return "open"
    except OSError as e: return "closed"
for host, port in [("${hostSide}", ${proxyPort}), ("${hostSide}", 22), ("${hostSide}", 8000), ("${hostSide}", 53), ("1.1.1.1", 443), ("169.254.169.254", 80), ("172.17.0.1", 80), ("127.0.0.1", 8000), ("${otherHost}", ${proxyPort}), ("${net}.${state().sandboxes[other].netidx}.2", 22)]:
    print(host, port, tcp(host, port))
try:
    socket.getaddrinfo("example.com", 443); print("dns works")
except OSError: print("dns fails")
u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); u.settimeout(2)
try:
    u.sendto(b"x", ("1.1.1.1", 53)); u.recvfrom(10); print("udp answered")
except OSError: print("udp fails")
print("ipv6", open("/proc/net/if_inet6").read().strip() or "none")
print("routes", len(open("/proc/net/route").read().strip().split("\\n")) - 1)
`;
		const result = await call(work, `python3 - <<'EOF'\n${probe}\nEOF\nprintf 'CONNECT example.com:443 HTTP/1.1\\r\\n\\r\\n' | timeout 5 python3 -c 'import os,socket,sys; h,p=os.environ["HTTPS_PROXY"].rsplit("@",1)[1].split(":"); s=socket.create_connection((h,int(p)),timeout=3); s.sendall(sys.stdin.buffer.read()); print(s.recv(100).decode().split(chr(13))[0])'`);
		const lines = result.stdout.trim().split("\n");
		assert.equal(lines[0], `${hostSide} ${proxyPort} open`, result.stdout + result.stderr);
		for (const line of lines.slice(1, 10)) assert.match(line, / closed$/, line);
		assert.deepEqual(lines.slice(10), ["dns fails", "udp fails", "ipv6 none", "routes 1", "HTTP/1.1 403 Forbidden"]);
		assert.ok(seen.includes("CONNECT example.com:443 HTTP/1.1"), String(seen));
	} finally {
		proxy.close();
	}
});

test("one thread cannot see another thread's files, processes, network or call secrets", { skip }, async () => {
	const workA = newWork();
	const claimsA = claimsFor(workA);
	await call(workA, "echo secret-a > /work/a.txt; echo tmp-a > /tmp/a.txt; setsid sleep 600 >/dev/null 2>&1 & echo started", { claims: claimsA });
	const b = await call(
		newWork(),
		`find / -xdev \\( -name a.txt -o -name '${workA}*' \\) 2>/dev/null; grep -rls secret-a /work /tmp 2>/dev/null; ps -eo comm= | sort | uniq -c | tr -s ' '; cat /proc/*/environ 2>/dev/null | tr '\\0' '\\n' | grep -c '${claimsA.proxy_password}'`,
	);
	assert.doesNotMatch(b.stdout, /a\.txt|secret-a/);
	assert.doesNotMatch(b.stdout, /sleep\n.*sleep/s);
	assert.match(b.stdout.trim(), /\n0$/, "B cannot read A's proxy password");
	// The same thread: its files, its background process, and a new token per call.
	const again = claimsFor(workA);
	const a2 = await call(workA, `cat /work/a.txt /tmp/a.txt; pgrep -cf '^sleep 600'; echo "$ORBIT_TOKEN"`, { claims: again });
	assert.deepEqual(a2.stdout.trim().split("\n"), ["secret-a", "tmp-a", "1", again.token]);
	assert.notEqual(again.token, claimsA.token);
});

test("tickets: a wrong key, an expired ticket, a replay and a changed limit are refused", { skip }, async () => {
	const work = newWork();
	const wrong = await request({ op: "run", ticket: ticket(claimsFor(work), randomBytes(32)), script: "echo ran" });
	assert.equal(wrong.error, "invalid ticket");
	const expired = await call(work, "echo ran", { claims: claimsFor(work, { exp: Date.now() - 1 }) });
	assert.equal(expired.error, "the ticket expired");
	const claims = claimsFor(work);
	const signed = ticket(claims);
	assert.equal((await request({ op: "run", ticket: signed, script: "echo ran" })).stdout, "ran\n");
	assert.equal((await request({ op: "run", ticket: signed, script: "echo ran" })).error, "the ticket was used");
	const [, payload, mac] = ticket(claimsFor(work)).split(".");
	const changed = JSON.parse(Buffer.from(payload, "base64url").toString("utf8"));
	changed.limits.memory_mb = 8192;
	const forged = `v1.${Buffer.from(JSON.stringify(changed)).toString("base64url")}.${mac}`;
	assert.equal((await request({ op: "run", ticket: forged, script: "echo ran" })).error, "invalid ticket");
	assert.equal((await request({ op: "run", script: "echo ran" })).error, "invalid ticket");
});

test("a fork bomb stops at RLIMIT_NPROC; the sandbox lives and the next call works", { skip }, async () => {
	const work = newWork();
	await call(work, "setsid sleep 600 >/dev/null 2>&1 &");
	const { cgroup } = state().sandboxes[work];
	const bomb = await call(work, "bomb() { bomb | bomb & }; bomb; sleep 5; echo survived", { limits: { timeout_s: 15 } });
	assert.ok(bomb.timed_out || bomb.exit !== null);
	const tasks = Number(readFileSync(`/sys/fs/cgroup${cgroup}/pids.peak`, "utf8"));
	assert.ok(tasks < 1536, `host tasks ${tasks} stayed below the slice backstop`);
	const next = await call(work, "pgrep -cf '^sleep 600'; echo ok", { limits: { timeout_s: 30 } });
	assert.equal(next.stdout, "1\nok\n", next.stderr);
});

test("memory stops at 1 GiB: only the greedy process dies; the sandbox and its other processes live", { skip }, async () => {
	const work = newWork();
	await call(work, "setsid sleep 600 >/dev/null 2>&1 &");
	const { cgroup } = state().sandboxes[work];
	const greedy = await call(work, "python3 -c 'b = bytearray(2 * 1024**3); print(len(b))'");
	assert.notEqual(greedy.exit, 0);
	assert.doesNotMatch(greedy.stdout, /2147483648/);
	const peak = Number(readFileSync(`/sys/fs/cgroup${cgroup}/memory.peak`, "utf8"));
	assert.ok(peak <= 1024 * 1024 * 1024, `memory.peak ${peak}`);
	const next = await call(work, "pgrep -cf '^sleep 600'");
	assert.equal(next.stdout, "1\n");
});

test("a disk fill stops at the /work image size", { skip }, async () => {
	const work = newWork();
	const result = await call(work, `dd if=/dev/zero of=/work/fill bs=1M count=${quotaMb * 2} status=none; echo dd=$?; du -m /work/fill | cut -f1; df -m --output=size /work | tail -1; rm -f /work/fill`);
	const [status, used, size] = result.stdout.trim().split("\n");
	assert.notEqual(status, "dd=0");
	assert.match(result.stderr, /No space left|File too large/);
	assert.ok(Number(used) <= Number(size) && Number(size) <= quotaMb, `${used} MiB used of ${size}`);
});

test("the call timeout kills the call's process group, not the sandbox or a detached process", { skip }, async () => {
	const work = newWork();
	const started = Date.now();
	const result = await call(work, "setsid sleep 301 >/dev/null 2>&1 & (sleep 302 &); sleep 303", { limits: { timeout_s: 3 } });
	assert.equal(result.timed_out, true);
	assert.ok(Date.now() - started < 15_000);
	const left = await call(work, "ps -eo args= | grep -E '^sleep 30[0-9]' | sort");
	// `sleep 302` was a child of the call's subshell, in the call's group: gone. The setsid one stays (by design).
	assert.equal(left.stdout, "sleep 301\n");
});

test("zombies are reaped by tini", { skip }, async () => {
	const work = newWork();
	await call(work, "for i in $(seq 40); do (sleep 0.01 &) ; done; sleep 1");
	const result = await call(work, "ps -eo stat= | grep -c '^Z' || true");
	assert.equal(result.stdout, "0\n");
});

test("the broker reports each call end to Orbit with the service key and passes on waiting egress requests", { skip }, async () => {
	const claims = claimsFor(newWork());
	const result = await call(claims.work, "exit 3", { claims });
	assert.equal(result.exit, 3);
	assert.deepEqual(result.egress_pending, [{ host: "api.example.com", port: 443 }]);
	const report = ends.find((end) => end.path === `/api/internal/agent/sandbox/calls/${claims.call}/end`);
	assert.ok(report, "the broker reported the call end");
	assert.equal(report.auth, `Bearer ${derived("orbit-sandbox/service").toString("hex")}`);
	assert.equal(report.body.exit, 3);
	assert.equal(report.body.timed_out, false);
	assert.doesNotMatch(JSON.stringify(report.body), new RegExp(claims.token));
});

test("eviction: a 4th sandbox deletes the one quiet the longest, never one with a running call; /work stays", { skip }, async () => {
	await request({ op: "stop_all" });
	const [a, b, c, d] = [newWork(), newWork(), newWork(), newWork()];
	const busy = call(a, "sleep 6; echo a-done", { limits: { timeout_s: 30 } }); // the oldest call, still running
	await sleep(1500);
	await call(b, "echo kept-b > /work/b.txt");
	await call(c, "true");
	await call(d, "true");
	assert.deepEqual(live().sort(), [a, c, d].sort(), "b (quiet the longest, no running call) was evicted, not a");
	assert.equal((await busy).stdout, "a-done\n");
	const back = await call(b, "cat /work/b.txt");
	assert.equal(back.stdout, "kept-b\n", "the evicted thread's /work stayed");
	assert.equal(live().length, 3);
});

test("eviction: with all 3 sandboxes busy, a 4th call waits, then fails with busy", { skip, timeout: 90_000 }, async () => {
	await request({ op: "stop_all" });
	const works = [newWork(), newWork(), newWork()];
	const busy = works.map((work) => call(work, "sleep 45", { limits: { timeout_s: 60 } }));
	await sleep(3000);
	const started = Date.now();
	const fourth = await call(newWork(), "echo ran");
	assert.equal(fourth.error, "all sandboxes are busy, try again");
	assert.ok(Date.now() - started >= 29_000);
	await request({ op: "stop_all" });
	for (const result of await Promise.all(busy)) assert.equal(result.interrupted, "off switch");
});

test("gc: idle sandboxes go (not busy ones); max life deletes a busy one and its call ends as interrupted", { skip, timeout: 60_000 }, async () => {
	await request({ op: "stop_all" });
	const [idle, busyWork, old] = [newWork(), newWork(), newWork()];
	await call(idle, "echo kept > /work/k.txt", { limits: { idle_minutes: 5 } });
	const busy = call(busyWork, "sleep 8; echo busy-done", { limits: { timeout_s: 30, idle_minutes: 5 } });
	const oldCall = call(old, "sleep 20; echo never", { limits: { timeout_s: 30 } });
	await sleep(2000);
	editState(`for w in ("${idle}", "${busyWork}"): s["sandboxes"][w]["last_call"] = time.time() - 301\ns["sandboxes"]["${old}"]["started"] = time.time() - 5 * 3600`);
	await request({ op: "gc" });
	assert.deepEqual(live(), [busyWork], "idle and max-life sandboxes deleted; the busy one kept");
	assert.equal((await busy).stdout, "busy-done\n");
	const interrupted = await oldCall;
	assert.equal(interrupted.interrupted, "the sandbox reached its maximum life");
	assert.equal((await call(idle, "cat /work/k.txt")).stdout, "kept\n");
});

test("off switch: stop_agent kills one agent's sandboxes (busy and idle); stop_all the rest; nothing is left", { skip, timeout: 60_000 }, async () => {
	await request({ op: "stop_all" });
	const [mine, mineBusy, theirs] = [newWork(), newWork(), newWork()];
	await call(mine, "echo kept > /work/k.txt; setsid sleep 600 >/dev/null 2>&1 &");
	const busy = call(mineBusy, "sleep 30", { limits: { timeout_s: 60 } });
	await call(theirs, "true", { agent: OTHER_AGENT });
	await sleep(1000);
	const started = Date.now();
	const stopped = await request({ op: "stop_agent", agent: AGENT });
	assert.ok(Date.now() - started < 5000);
	assert.equal((stopped as unknown as { stopped: number }).stopped, 2);
	assert.deepEqual(live(), [theirs]);
	assert.equal((await busy).interrupted, "off switch");
	await request({ op: "stop_all" });
	assert.deepEqual(live(), []);
	assert.equal(scopes(), "");
	assert.deepEqual(netns(), []);
	assert.equal(containers(), "");
	assert.equal(count("pgrep", ["-fc", `runsc.*--root=${run}/runsc`]), "0");
	assert.equal((await call(mine, "cat /work/k.txt; pgrep -cf '^sleep 600' || true")).stdout, "kept\n0\n", "/work kept, processes gone");
});

test("a broker restart (socket start) deletes every sandbox and its state", { skip, timeout: 60_000 }, async () => {
	await call(newWork(), "setsid sleep 600 >/dev/null 2>&1 &");
	sh("systemctl", ["restart", `${name}.socket`]);
	assert.deepEqual(live(), []);
	assert.equal(scopes(), "");
	assert.deepEqual(netns(), []);
	assert.equal(containers(), "");
	assert.doesNotMatch(sh("ip", ["-o", "link"]), new RegExp(`${netPrefix}h\\d`));
});
