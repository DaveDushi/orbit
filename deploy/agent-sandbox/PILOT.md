# Arthur pilot: production steps (phase 10, Part B)

For Jean and Ops. **Prepared, not run.** Each block needs the owner's yes. The design is
[`docs/agent-sandbox-design.md`](../../docs/agent-sandbox-design.md); the test copy that proved these steps is in its
section 20.

Order: the code deploy (step 1) is a normal deploy, with the sandbox still off. The switch-on (steps 2–7) is a separate
change on another day. Each step can be undone alone (step 9).

| Name | Production value |
|---|---|
| Orbit | `orbit.service`, user `orbit`, `127.0.0.1:8080` (`ORBIT_BIND`, `ORBIT_PORT`); nginx `:8000` in front |
| Sidecar | `orbit-agent.service`, user and group `orbit-agent`, env `/etc/orbit/agent.env` |
| Broker | `orbit-sandbox.socket` → `/run/orbit-sandbox/sandbox.sock` (root, group `orbit-agent`, 0660) |
| Proxy | `orbit-sandbox-egress.service`, user `orbit-sandbox-egress`, port 3128 on each sandbox's /30 host address |
| Key | `/etc/orbit/sandbox-broker.key` (root:orbit 0640: the broker and Orbit; never the sidecar or the proxy) |
| Network | `10.201.0.0/16`, netns `osb-*`, veth `osbh*`/`osbp*`, nft table `inet orbit_sandbox` |
| gVisor | `release-20261109.0`, SHA-512 pinned in `setup.sh`, in `/usr/local/lib/orbit-sandbox/gvisor` |

## 1. Code deploy (sandbox still off)

Normal deploy of the merge (after local `main` 5b0d9803 and `external-agents-push`). **No migration** in this branch:
the new route `POST /api/internal/agent/files` (sidecar secret) and the `file_ids` field need no schema change. The
migration dry run of the normal deploy covers the migrations that come from `main`. Nothing changes for people: without
`ORBIT_AGENT_SANDBOX_KEY_FILE` every sandbox route answers 404 `sandbox_off`, and without
`AGENT_SANDBOX_BROKER_SOCKET` no agent gets `sandbox_shell`. Checks: `/root/ops/agent-sandbox/routes.sh` (404
`sandbox_off`, 422 or 401, never 200).

## 2. Host guards (phase 2): check only

They are live since 2026-11-16 (`deploy/host-guards/README.md`). Check, change nothing:

```sh
systemctl show orbit.slice -p MemoryHigh -p MemoryMax          # 11811160064 / 12884901888
systemctl show orbit-agents.slice -p MemoryMax -p TasksMax -p CPUQuotaPerSecUSec  # 2684354560 / 1536 / 1.5s
systemctl show orbit.service -p OOMScoreAdjust -p MemoryMin    # -800 / 536870912
systemctl show orbit-agent.service -p MemoryMin                # 268435456
sudo bash /root/ops/vm-health                                  # guards OK
```

When a value differs: apply `deploy/host-guards/` as its README says (no restart), then check again.

## 3. Agent image and broker + proxy

Build the image with the limited builder (never `/usr/bin/docker build`), then install. `setup.sh` is safe to run
again; it does not start a sandbox.

```sh
cd /home/exedev/orbit                     # the deployed revision
REV=$(git rev-parse --short HEAD)
orbit-build /usr/bin/docker buildx build --builder=orbit-limited --load -t orbit-agent-sandbox:$REV deploy/agent-sandbox
sudo deploy/agent-sandbox/setup.sh \
  --client-group orbit-agent --key-group orbit \
  --image orbit-agent-sandbox:$REV \
  --orbit-url http://127.0.0.1:8080 --egress-bin /usr/local/bin/orbit
# defaults: --name orbit-sandbox --slice-prefix orbit-agents --net 10.201 --net-prefix osb --proxy-port 3128
#           --quota-mb 4096 --min-free-mb 8192 --key /etc/orbit/sandbox-broker.key
```

Check:

```sh
/usr/local/lib/orbit-sandbox/gvisor/runsc --version | head -1      # runsc version release-20261109.0
stat -c '%A %U:%G' /etc/orbit/sandbox-broker.key                     # -rw-r----- root:orbit
stat -c '%A %U:%G' /run/orbit-sandbox/sandbox.sock                   # srw-rw---- root:orbit-agent
systemctl is-active orbit-sandbox.socket orbit-sandbox-gc.timer orbit-sandbox-egress.service
nft list table inet orbit_sandbox | grep -c 'dport 3128'             # 2
cat /var/lib/orbit-sandbox/rootfs/current.id                         # the image id: write it in the deploy note
docker image rm orbit-agent-sandbox:$REV                             # the rootfs is exported; the image is not needed
```

`setup.sh` copies the orbit binary for the proxy (`/usr/local/lib/orbit-sandbox/orbit`): after each later Orbit deploy,
run `setup.sh` again with the same options, so the proxy runs the same version as Orbit.

The proxy and the broker call Orbit on `127.0.0.1:8080` with `X-Forwarded-Proto: https` (phase 9, finding 3; the
sidecar already sends it: `ORBIT_FORWARDED_PROTO=https` in `/etc/orbit/agent.env`). Orbit trusts this header only from
`ORBIT_TRUSTED_PROXIES=127.0.0.1/32,::1/128`, as now.

## 4. Orbit: the broker key

```sh
sudo cp /etc/orbit/orbit.env /etc/orbit/orbit.env.before-sandbox-pilot
echo 'ORBIT_AGENT_SANDBOX_KEY_FILE=/etc/orbit/sandbox-broker.key' | sudo tee -a /etc/orbit/orbit.env
sudo -u orbit test -r /etc/orbit/sandbox-broker.key && echo readable
sudo systemctl restart orbit                    # ~5 s down; the sidecar retries
curl -fsS -H 'X-Forwarded-Proto: https' http://127.0.0.1:8080/health/ready
```

Now the sandbox routes answer (owners and admins see the Sandbox fields on the agent card), but no agent has
`sandbox_shell` ticked, so nothing runs.

## 5. Sidecar: the broker socket

```sh
sudo install -m 0644 /dev/stdin /etc/systemd/system/orbit-agent.service.d/orbit-sandbox.conf <<'UNIT'
[Service]
Environment=AGENT_SANDBOX_BROKER_SOCKET=/run/orbit-sandbox/sandbox.sock
UNIT
sudo systemctl daemon-reload && sudo systemctl restart orbit-agent
curl -fsS -H "Authorization: Bearer $(sudo grep -oP '^ORBIT_AGENT_SECRET=\K.*' /etc/orbit/agent.env)" \
  http://127.0.0.1:8790/health                                     # {"ok":true,…,"sandbox":true}
```

(The sidecar keeps `AGENT_SANDBOX_SOCKET` of the media sandbox; it is a different socket.)

## 6. Optional hardening: hide other users' processes (defense in depth for finding 7)

Finding 7 is fixed in the broker (the call token, the proxy password and the script go in a root-only `--process` spec
file, never on a command line). As a second layer, Ops may hide processes per service. Do not mount the whole `/proc`
with `hidepid` on this VM (polkit, logind and monitoring need it); instead:

```sh
for unit in orbit orbit-agent orbit-sandbox-egress; do
  sudo install -D -m 0644 /dev/stdin /etc/systemd/system/$unit.service.d/protect-proc.conf <<'UNIT'
[Service]
ProtectProc=invisible
UNIT
done
sudo systemctl daemon-reload   # takes effect at each unit's next restart
```

Undo: remove the three `protect-proc.conf` files, `daemon-reload`, restart.

## 7. Switch on for Arthur (the owner, in the app)

Settings → Agentic → Agents → Arthur → Edit: tick **Sandbox** (`sandbox_shell`), Save; in the Sandbox section turn it
on and keep the pilot limits (600 CPU-minutes, 2 GiB traffic, 20 M model tokens a month; idle 30 min). Internet access
page: start empty (every host asks). Then a first call in a DM with Arthur: "run `python3 -c 'print(2+2)'` in your
sandbox". Expected: `4`, a call row in the Internet access page's log only when it used the internet.

What's new (add at the deploy that switches it on, under that day):
`- Arthur (pilot) can now run code in its own sandbox: ask it to compute, convert or chart something, and it posts the result file in the thread. When it needs a website, it asks you first with Allow once / Deny (owners and admins can also Allow always on Arthur's Internet access page).`

## 8. Watch (2–4 weeks; exit criteria in design section 8)

`sudo bash /root/ops/vm-health` daily; `journalctl -u 'orbit-sandbox@*' -u orbit-sandbox-egress --since today`
(short reasons only, no tokens); the agent card's monthly use; the Internet access page's log (narrow hosts only, never a
CDN's shared name); calls ending `interrupted` or `lost`; `systemctl show orbit-agents.slice -p MemoryPeak`; OOM kills
only inside `orbit.slice`.

## 9. Rollback (each line alone; no data step)

| Level | How | Effect |
|---|---|---|
| Arthur only | Untick Sandbox on Arthur's card (or turn its sandbox off) | Running calls stop at once ("the sandbox was switched off"); no new calls. `/work` stays 72 h. |
| Installation | `sudo sed -i '/^ORBIT_AGENT_SANDBOX_KEY_FILE=/d' /etc/orbit/orbit.env && sudo systemctl restart orbit` | Every sandbox route 404 `sandbox_off`; the sidecar's next poll stops all sandboxes. |
| Sidecar | `sudo rm /etc/systemd/system/orbit-agent.service.d/orbit-sandbox.conf && sudo systemctl daemon-reload && sudo systemctl restart orbit-agent` | No agent gets `sandbox_shell`. |
| Emergency | `sudo systemctl stop orbit-sandbox.socket orbit-sandbox-egress.service; sudo systemctl kill orbit-agents.slice` | Nothing runs, nothing leaves. Orbit and chat keep working. |
| Remove all | `sudo deploy/agent-sandbox/setup.sh --uninstall` (after the installation line) | Units, gVisor, rootfs, `/work` images, nft table, proxy user removed; the key under `/etc/orbit` stays (delete it by hand). |
| Code | Previous orbit binary and sidecar `src`, as in each deploy note | No schema change in this branch, so no data step. |
