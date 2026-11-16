# Host guards (agent sandbox phase 2)

Applied on the Orbit VM on 2026-11-16 without an Orbit restart. See docs/agent-sandbox-design.md section 6.

| File | Install path |
|---|---|
| orbit.slice | /etc/systemd/system/orbit.slice (MemoryHigh 11G, MemoryMax 12G; parent of orbit-builds, orbit-donna, orbit-agents) |
| orbit-agents.slice | /etc/systemd/system/orbit-agents.slice (2G/2560M, swap 0, 1536 tasks, CPU 150%) |
| orbit.service-oom.conf | /etc/systemd/system/orbit.service.d/oom.conf (OOMScoreAdjust -800, MemoryMin 512M) |
| orbit-agent.service-oom.conf | /etc/systemd/system/orbit-agent.service.d/oom.conf (MemoryMin 256M) |
| vm-health | /usr/local/bin/vm-health (Ops copy; adds guards, slice OOM and pressure checks) |

Then: `systemctl daemon-reload` and `choom -p "$(systemctl show -p MainPID --value orbit)" -n -800`.
Rollback (no restart): `rollback.sh`.
