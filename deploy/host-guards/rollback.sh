#!/usr/bin/env bash
# Roll back agent sandbox phase 2 host guards. No service restart needed.
set -eux
rm -f /etc/systemd/system/system.slice.d/memorymin.conf
rm -f /etc/systemd/system/orbit.slice /etc/systemd/system/orbit-agents.slice \
      /etc/systemd/system/orbit.service.d/oom.conf /etc/systemd/system/orbit-agent.service.d/oom.conf
systemctl daemon-reload
systemctl set-property --runtime orbit.slice MemoryHigh=infinity MemoryMax=infinity
systemctl set-property --runtime system.slice MemoryMin=0
systemctl set-property --runtime orbit.service MemoryMin=0
systemctl set-property --runtime orbit-agent.service MemoryMin=0
rm -rf /run/systemd/system.control/orbit.slice.d /run/systemd/system.control/system.slice.d /run/systemd/system.control/orbit.service.d /run/systemd/system.control/orbit-agent.service.d
systemctl daemon-reload
choom -p "$(systemctl show -p MainPID --value orbit.service)" -n 0
