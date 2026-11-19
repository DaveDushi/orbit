echo "--- find A files:"; find / -xdev -name a.txt 2>/dev/null | head; grep -rl secretA /work /tmp 2>/dev/null | head -3
echo "--- host paths:"; for p in /etc/orbit /var/lib/orbit /root/.claude /home/exedev /run/docker.sock /opt/orbit-spike /etc/shadow; do [ -e $p ] && echo "EXISTS $p" || echo "missing $p"; done; head -c 40 /etc/shadow 2>&1; echo
echo "--- kernel:"; uname -r
echo "--- mounts:"; cat /proc/mounts
echo "--- net:"; tail -n +3 /proc/net/dev | cut -c1-12; cat /proc/net/route
timeout 5 wget -q -T 3 -O- http://169.254.169.254/latest/meta-data/ 2>&1 | head -2; timeout 5 wget -q -T 3 -O- http://127.0.0.1:8000/health 2>&1 | head -1; timeout 5 nc -w 2 1.1.1.1 443 </dev/null; echo "nc exit=$?"; timeout 5 nslookup example.com 2>&1 | tail -2
echo "--- privileged ops:"; mount -t tmpfs x /mnt 2>&1; unshare -U true 2>&1; echo 1 > /proc/sys/kernel/sysrq 2>&1; touch /bin/x 2>&1; timeout 3 su -c id </dev/null 2>&1 | head -1
echo "--- host processes:"; ps -o pid,args | wc -l; ls /proc | grep -c '^[0-9]'
echo "--- env:"; env | sort
