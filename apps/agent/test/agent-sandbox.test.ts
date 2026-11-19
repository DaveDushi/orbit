/**
 * Escape, limit and lifecycle tests of the agent sandbox broker (deploy/agent-sandbox/broker.py: one gVisor sandbox
 * per chat thread), against a real broker. Run them as root, only on a TEST COPY (never production):
 *
 *   deploy/agent-sandbox/setup.sh --name orbit-sbx-test --slice-prefix orbit-sbx-test --net 10.202 --net-prefix sbt \
 *     --proxy-port 13128 --quota-mb 256 --min-free-mb 4096 --client-group root --image <agent image> \
 *     --key /etc/orbit-sbx-test/broker.key --orbit-url http://127.0.0.1:13129 [--egress-bin <orbit binary>]
 *   AGENT_SBX_TEST_NAME=orbit-sbx-test node --test test/agent-sandbox.test.ts
 *
 * The tests stand in for Orbit: they sign tickets with the test key and answer the broker's call-end reports. With
 * --egress-bin the real egress proxy runs; the stand-in then also decides egress (allow example.com and *.nip.io, ask
 * for the rest, deny IP literals), takes the byte reports, and answers for orbit.internal. The proxy tests need DNS
 * and HTTPS from the host (example.com; 10.0.0.1.nip.io for a name that resolves to a private address).
 */

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHmac, randomBytes, randomUUID } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
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
const realProxy = Boolean(env.SBX_EGRESS_GROUP);
const skipProxy = skip || (realProxy ? false : "the test copy runs no egress proxy (setup.sh --egress-bin)");
const key = name ? readFileSync(env.SBX_KEY_FILE).toString("utf8").trim() : "";
const derived = (label: string) => createHmac("sha256", key).update(label).digest();

const AGENT = "0190a0b0-0000-7000-8000-00000000000a";
const OTHER_AGENT = "0190a0b0-0000-7000-8000-00000000000b";
const newWork = () => randomBytes(16).toString("hex");

type Limits = { timeout_s?: number; memory_mb?: number; cpu?: number; idle_minutes?: number; disk_mb?: number };
type Claims = { call: string; agent: string; work: string; exp: number; limits: Limits; token: string; proxy_password: string };
type Result = { exit: number | null; timed_out: boolean; interrupted: string | null; restarted?: string | null; error?: string; egress_pending?: unknown[]; cpu_ms?: number; stdout: string; stderr: string };

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

/** A call id as Orbit makes it (UUID v7; the egress proxy takes no other). */
function uuidv7(): string {
	const bytes = randomBytes(16);
	bytes.writeUIntBE(Date.now(), 0, 6);
	bytes[6] = (bytes[6] & 0x0f) | 0x70;
	bytes[8] = (bytes[8] & 0x3f) | 0x80;
	const hex = bytes.toString("hex");
	return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/** Every call's proxy password, for the stand-in's egress check. */
const passwords = new Map<string, string>();

function claimsFor(work: string, options: { agent?: string; limits?: Limits; exp?: number } = {}): Claims {
	const claims = {
		call: uuidv7(),
		agent: options.agent ?? AGENT,
		work,
		exp: options.exp ?? Date.now() + 300_000,
		limits: { timeout_s: 60, memory_mb: 1024, cpu: 1, idle_minutes: 30, ...options.limits },
		token: `orbit-agent.${randomBytes(12).toString("base64url")}`,
		proxy_password: randomBytes(12).toString("base64url"),
	};
	passwords.set(claims.call, claims.proxy_password);
	return claims;
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

// Stands in for Orbit: the call-end route (records the reports and answers one waiting egress request), the egress
// check and byte reports of the proxy, and any other path as orbit.internal (echoes what it got).
const ends: { path: string; auth: string; body: Record<string, unknown> }[] = [];
const checks: { call_id: string; host: string; port: number; decision: string }[] = [];
const dones: { log_id: string; call_id: string; bytes_up: number; bytes_down: number; blocked: string | null }[] = [];
const endedCalls = new Set<string>();
/** The service calls of the broker and the proxy, with their X-Forwarded-Proto (production Orbit wants https). */
const serviceProtos: string[] = [];
/** The paths that reached the stand-in as orbit.internal requests. */
const seenPaths: string[] = [];
const decide = (host: string) => (/^[\d.]+$|[:[]/.test(host) || host === "orbit.internal" ? "deny" : host === "example.com" || host.endsWith(".nip.io") ? "allow" : "ask");
let orbit: HttpServer | undefined;
before(async () => {
	if (skip) return;
	await request({ op: "stop_all" });
	orbit = createServer((req, res) => {
		let body = "";
		req.on("data", (chunk) => (body += chunk));
		req.on("end", () => {
			const path = req.url ?? "";
			const json = (status: number, value: unknown) => {
				res.writeHead(status, { "content-type": "application/json" });
				res.end(JSON.stringify(value));
			};
			const service = req.headers.authorization === `Bearer ${derived("orbit-sandbox/service").toString("hex")}`;
			if (path.startsWith("/api/internal/")) serviceProtos.push(`${path.split("/").slice(3, 5).join("/")} ${req.headers["x-forwarded-proto"]}`);
			if (path === "/api/internal/agent/egress/check") {
				const asked = JSON.parse(body);
				if (!service || passwords.get(asked.call_id) !== asked.proxy_password || endedCalls.has(asked.call_id)) return json(403, { error: "egress_call_unknown" });
				const host = String(asked.host).toLowerCase().replace(/\.$/, "");
				const decision = decide(host);
				checks.push({ call_id: asked.call_id, host, port: asked.port, decision });
				return json(200, { decision, reason: "stand-in", host: decision === "deny" ? null : host, log_id: randomUUID() });
			}
			if (path === "/api/internal/agent/egress/done") {
				if (!service) return json(401, {});
				dones.push(JSON.parse(body));
				return json(200, { counted: true });
			}
			if (path.startsWith("/api/internal/agent/sandbox/calls/")) {
				ends.push({ path, auth: req.headers.authorization ?? "", body: JSON.parse(body || "{}") });
				endedCalls.add(path.split("/")[6]);
				return json(200, { egress_pending: [{ host: "api.example.com", port: 443 }] });
			}
			seenPaths.push(path);
			json(200, { orbit_internal: true, method: req.method, path, headers: req.headers, body });
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
	assert.deepEqual(names(callEnv), ["HOME", "HTTPS_PROXY", "HTTP_PROXY", "LANG", "ORBIT_TOKEN", "ORBIT_URL", "PATH", "PWD", "PYTHONUSERBASE", "SHLVL", "TMPDIR", "_", "http_proxy", "https_proxy", "npm_config_prefix"]);
	assert.ok(callEnv.includes(`ORBIT_TOKEN=${claims.token}\n`));
	assert.ok(callEnv.includes(`HTTPS_PROXY=http://${claims.call}:${claims.proxy_password}@`));
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
	// Without the real proxy, a stub on the proxy port answers 403; the real one wants the call's password (407).
	const seen: string[] = [];
	const proxy = createTcpServer((socket) => socket.once("data", (data) => (seen.push(data.toString("utf8").split("\r\n")[0]), socket.end("HTTP/1.1 403 Forbidden\r\n\r\n"))));
	if (!realProxy) await new Promise<void>((resolve) => proxy.listen(proxyPort, hostSide, resolve));
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
		const answer = realProxy ? "HTTP/1.1 407 Proxy Authentication Required" : "HTTP/1.1 403 Forbidden";
		assert.deepEqual(lines.slice(10), ["dns fails", "udp fails", "ipv6 none", "routes 1", answer]);
		if (!realProxy) assert.ok(seen.includes("CONNECT example.com:443 HTTP/1.1"), String(seen));
	} finally {
		if (!realProxy) proxy.close();
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

test("a setsid fork bomb fills RLIMIT_NPROC; the next call restarts the sandbox, keeps /work and works", { skip, timeout: 120_000 }, async () => {
	const work = newWork();
	const bomb = await call(work, "echo kept > /work/kept; setsid bash -c 'bomb() { bomb | bomb & }; bomb' >/dev/null 2>&1 & sleep 5; echo started", { limits: { timeout_s: 20 } });
	assert.equal(bomb.restarted, null);
	const { cgroup } = state().sandboxes[work];
	assert.ok(Number(readFileSync(`/sys/fs/cgroup${cgroup}/pids.current`, "utf8")) >= 128, "the bomb outlived its call");
	const next = await call(work, "cat /work/kept; ps -eo comm= | grep -c bash", { limits: { timeout_s: 30 } });
	assert.equal(next.exit, 0, next.stderr);
	assert.match(String(next.restarted), /process limit was full/);
	assert.equal(next.stdout, "kept\n1\n");
	const again = await call(work, "echo again");
	assert.equal(again.restarted, null);
	assert.equal(again.stdout, "again\n");
});

// --- The egress proxy (a test copy with --egress-bin). Every request runs inside the sandbox with curl.

/** `curl` through the call's proxy: "<CONNECT status> <HTTP status> <exit>" per URL. */
const curl = (urls: string[]) =>
	urls.map((url) => `curl -s -o /dev/null --max-time 20 -w '%{http_connect} %{http_code} ' '${url}'; echo $?`).join("\n");

test("proxy: an allowed host works and its bytes are reported; others are asked or denied before any DNS", { skip: skipProxy, timeout: 120_000 }, async () => {
	const claims = claimsFor(newWork());
	const result = await call(claims.work, curl(["https://example.com/", "https://api.unknown.test/", "https://1.1.1.1/", "https://169.254.169.254/", "https://[::1]/", "http://example.com/"]), { claims });
	const lines = result.stdout.trim().split("\n");
	assert.equal(lines[0], "200 200 0", result.stdout + result.stderr);
	for (const line of lines.slice(1, 5)) assert.equal(line, "403 000 56", result.stdout);
	assert.equal(lines[5], "000 405 0", "plain HTTP is refused");
	const mine = checks.filter((check) => check.call_id === claims.call);
	assert.deepEqual(mine.map((check) => `${check.host} ${check.decision}`), ["example.com allow", "api.unknown.test ask", "1.1.1.1 deny", "169.254.169.254 deny", "[::1] deny"]);
	await sleep(500);
	const done = dones.find((report) => report.call_id === claims.call);
	assert.ok(done && done.bytes_down > 500 && done.bytes_up > 200 && done.blocked === null, JSON.stringify(dones));
});

test("proxy: names that resolve to private, loopback or metadata addresses are blocked after the yes (DNS rebinding)", { skip: skipProxy, timeout: 120_000 }, async () => {
	const claims = claimsFor(newWork());
	const hosts = ["10.0.0.1.nip.io", "127.0.0.1.nip.io", "169.254.169.254.nip.io", "192.168.1.1.nip.io", "172.16.0.1.nip.io"];
	const result = await call(claims.work, curl(hosts.map((host) => `https://${host}/`)), { claims });
	for (const line of result.stdout.trim().split("\n")) assert.equal(line, "403 000 56", result.stdout + result.stderr);
	await sleep(500);
	const blocked = dones.filter((report) => report.call_id === claims.call);
	assert.equal(blocked.length, hosts.length);
	for (const report of blocked) assert.match(String(report.blocked), /non-public/);
});

test("proxy: orbit.internal gets one request with the call token, the sandbox's address and no proxy password", { skip: skipProxy }, async () => {
	const claims = claimsFor(newWork());
	const script = `curl -s --max-time 10 -H "Authorization: Bearer $ORBIT_TOKEN" -H 'X-Forwarded-For: 1.2.3.4' -H 'X-Forwarded-Proto: http' --data '{"a":1}' "$ORBIT_URL/api/v1/me?x=1"; echo
curl -s -o /dev/null --max-time 10 -w '%{http_connect}' -p "$ORBIT_URL/"; echo`;
	const result = await call(claims.work, script, { claims });
	const [first, tunnel] = result.stdout.trim().split("\n");
	const seen = JSON.parse(first);
	assert.equal(seen.path, "/api/v1/me?x=1");
	assert.equal(seen.body, '{"a":1}');
	assert.equal(seen.headers.authorization, `Bearer ${claims.token}`);
	assert.equal(seen.headers["proxy-authorization"], undefined);
	assert.equal(seen.headers["x-forwarded-for"], `${net}.${state().sandboxes[claims.work].netidx}.2`);
	assert.equal(seen.headers["x-forwarded-proto"], "https");
	assert.equal(seen.headers.connection, "close");
	assert.equal(tunnel, "403", "a raw tunnel to Orbit is refused");
});

test("proxy: a password dies with its call (background processes too), and never works from another sandbox", { skip: skipProxy, timeout: 120_000 }, async () => {
	const a = claimsFor(newWork());
	// A background process holds a tunnel open past the call's end, then tries again.
	const background = [
		"import base64, os, socket, time, urllib.parse",
		'proxy = urllib.parse.urlsplit(os.environ["HTTPS_PROXY"])',
		'auth = base64.b64encode(f"{proxy.username}:{proxy.password}".encode()).decode()',
		"def connect():",
		"    s = socket.create_connection((proxy.hostname, proxy.port), timeout=30)",
		'    s.sendall(f"CONNECT example.com:443 HTTP/1.1\\r\\nProxy-Authorization: Basic {auth}\\r\\n\\r\\n".encode())',
		'    return s, s.recv(200).decode().split("\\r\\n")[0]',
		'log = open("/work/bg.log", "a", buffering=1)',
		's, status = connect(); log.write(status + "\\n")',
		's.recv(1); log.write("closed\\n")',
		'time.sleep(1); log.write(connect()[1] + "\\n")',
	].join("\n");
	const first = await call(a.work, `cat > /work/bg.py <<'PY'\n${background}\nPY\nsetsid python3 /work/bg.py >/dev/null 2>&1 < /dev/null & sleep 1`, { claims: a });
	assert.equal(first.exit, 0, first.stderr);
	await sleep(4000);
	const later = await call(a.work, "cat /work/bg.log");
	assert.equal(later.stdout, "HTTP/1.1 200 Connection established\nclosed\nHTTP/1.1 407 Proxy Authentication Required\n");

	// While a call runs in one sandbox, another sandbox uses its password: 407.
	const running = claimsFor(newWork());
	const busy = call(running.work, "sleep 6", { claims: running });
	await sleep(1500);
	const b = claimsFor(newWork());
	const stolen = await call(b.work, `curl -s -o /dev/null -w '%{http_connect}' --max-time 10 -x "http://${running.call}:${running.proxy_password}@$(echo "$HTTPS_PROXY" | cut -d@ -f2)" https://example.com/`, { claims: b });
	assert.equal(stolen.stdout, "407");
	await busy;
});

test("phase 9: a bare LF in a request to orbit.internal never reaches Orbit (no smuggled header)", { skip: skipProxy }, async () => {
	const claims = claimsFor(newWork());
	const script = [
		"import base64, os, socket, urllib.parse",
		'proxy = urllib.parse.urlsplit(os.environ["HTTPS_PROXY"])',
		'auth = base64.b64encode(f"{proxy.username}:{proxy.password}".encode()).decode()',
		"for extra in ['X-A: 1\\nX-Forwarded-For: 1.2.3.4', 'X-A: 1\\nTransfer-Encoding: chunked', 'X-A: 1']:",
		"    s = socket.create_connection((proxy.hostname, proxy.port), timeout=10)",
		'    s.sendall(f"GET http://orbit.internal/phase9-lf HTTP/1.1\\r\\nHost: orbit.internal\\r\\nProxy-Authorization: Basic {auth}\\r\\nAuthorization: Bearer {os.environ[\'ORBIT_TOKEN\']}\\r\\n{extra}\\r\\n\\r\\n".encode())',
		'    print(repr(s.recv(12)))',
	].join("\n");
	const before = seenPaths.length;
	const result = await call(claims.work, `python3 - <<'PY'\n${script}\nPY`, { claims });
	assert.equal(result.stdout, "b''\nb''\nb'HTTP/1.1 200'\n", result.stderr);
	assert.deepEqual(seenPaths.slice(before), ["/phase9-lf"], "only the clean request reached Orbit");
	// Without the call token as the bearer (sign-in, registration mails): the proxy answers, Orbit never sees it.
	const bare = await call(claims.work, `curl -s -o /dev/null -w '%{http_code}' --max-time 10 -X POST -H "Cookie: s=$ORBIT_TOKEN" "$ORBIT_URL/phase9-register"`);
	assert.equal(bare.stdout, "401");
	assert.ok(!seenPaths.includes("/phase9-register"));
});

test("phase 9: a TLS name (SNI) other than the CONNECT host is blocked before a byte goes up", { skip: skipProxy, timeout: 60_000 }, async () => {
	const claims = claimsFor(newWork());
	// curl asks the proxy for example.com (allowed) but speaks TLS for www.iana.org (not allowed).
	const result = await call(claims.work, "curl -s -o /dev/null -w '%{http_connect} %{http_code}' --max-time 15 --connect-to www.iana.org:443:example.com:443 https://www.iana.org/; echo \" $?\"", { claims });
	assert.match(result.stdout.trim(), /^200 000 (35|56)$/, result.stdout + result.stderr);
	await sleep(500);
	const report = dones.find((done) => done.call_id === claims.call);
	assert.ok(report && /SNI/.test(String(report.blocked)) && report.bytes_up === 0, JSON.stringify(report));
});

test("phase 9: CPU time of background processes between calls counts at the next call", { skip, timeout: 60_000 }, async () => {
	const work = newWork();
	const first = await call(work, "setsid -f bash -c 'end=$((SECONDS+3)); while [ $SECONDS -lt $end ]; do :; done' </dev/null >/dev/null 2>&1; true");
	assert.equal(first.exit, 0, first.stderr);
	assert.ok((first.cpu_ms ?? 0) < 1500, `first call ${first.cpu_ms} ms`);
	await sleep(4500);
	const next = await call(work, "true");
	assert.ok((next.cpu_ms ?? 0) >= 2000, `the next call counted ${next.cpu_ms} ms`);
	const third = await call(work, "true");
	assert.ok((third.cpu_ms ?? 0) < 1000, `counted once: ${third.cpu_ms} ms`);
});

test("phase 9: the broker's and the proxy's calls to Orbit say https (production Orbit refuses plain loopback calls)", { skip: skipProxy }, async () => {
	const claims = claimsFor(newWork());
	const before = serviceProtos.length;
	await call(claims.work, "curl -s -o /dev/null --max-time 15 https://example.com/ https://api.unknown.test/", { claims });
	await sleep(500);
	const seen = [...new Set(serviceProtos.slice(before))].sort();
	assert.deepEqual(seen, ["agent/egress https", "agent/sandbox https"], JSON.stringify(serviceProtos.slice(before)));
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

test("a client that hangs up stops its call: killed, reported as interrupted, its proxy file gone", { skip }, async () => {
	const claims = claimsFor(newWork());
	const socket = createConnection({ path: socketPath });
	await new Promise((resolve) => socket.on("connect", resolve));
	socket.write(`${JSON.stringify({ op: "run", ticket: ticket(claims), script: "echo started; sleep 120" })}\n`);
	// The run is stopped (or the sidecar dies) once the call runs.
	await new Promise((resolve) => socket.once("data", resolve));
	socket.destroy();
	const path = `/api/internal/agent/sandbox/calls/${claims.call}/end`;
	const started = Date.now();
	while (!ends.some((end) => end.path === path) && Date.now() - started < 20_000) await new Promise((resolve) => setTimeout(resolve, 200));
	const report = ends.find((end) => end.path === path);
	assert.ok(report, "the broker reported the call end");
	assert.equal(report.body.interrupted, "the run stopped the call");
	assert.ok(Date.now() - started < 15_000);
	assert.equal(count("sh", ["-c", `ls ${run}/egress | grep -c ${claims.call}`]), "0");
	const left = await call(claims.work, "ps -eo args= | grep -c '^sleep 120' || true");
	assert.equal(left.stdout, "0\n");
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
	// About 3 s of CPU, then a long wait: the stop cuts it, and its CPU time until then still counts.
	const busyClaims = claimsFor(mineBusy, { limits: { timeout_s: 60 } });
	const busy = call(mineBusy, "timeout 3 sh -c 'while :; do :; done'; sleep 30", { claims: busyClaims });
	await call(theirs, "true", { agent: OTHER_AGENT });
	await sleep(4000);
	assert.ok(existsSync(`${run}/egress/${busyClaims.call}`), "the running call has its proxy file");
	const started = Date.now();
	const stopped = await request({ op: "stop_agent", agent: AGENT, reason: "the sandbox was switched off" });
	assert.ok(Date.now() - started < 5000);
	assert.equal((stopped as unknown as { stopped: number }).stopped, 2);
	assert.deepEqual(live(), [theirs]);
	assert.ok(!existsSync(`${run}/egress/${busyClaims.call}`), "gone with the stop: the proxy drops the call's tunnels");
	const cut = await busy;
	assert.equal(cut.interrupted, "the sandbox was switched off");
	assert.ok((cut.cpu_ms ?? 0) >= 2000, `the CPU time until the stop counts (${cut.cpu_ms} ms)`);
	const reported = ends.find((end) => end.path.includes(busyClaims.call));
	assert.equal(reported?.body.interrupted, "the sandbox was switched off");
	assert.equal(reported?.body.cpu_ms, cut.cpu_ms);
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
