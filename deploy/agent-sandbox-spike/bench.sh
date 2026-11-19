now() { awk '{printf "%d", $1*1000}' /proc/uptime; }
t() { s=$(now); "$@" >/dev/null 2>&1; echo $(( $(now) - s )); }
W=$1
echo "syscalls_dd_1M $(t dd if=/dev/zero of=/dev/null bs=1 count=500000)"
echo "spawn_300 $(t sh -c 'for i in $(seq 1 300); do /bin/true; done')"
echo "files_2000 $(t sh -c "mkdir -p $W/b && cd $W/b && for i in \$(seq 1 2000); do echo x > f\$i; done && cat f* >/dev/null && cd / && rm -rf $W/b")"
echo "cpu_awk $(t awk 'BEGIN{for(i=0;i<3000000;i++) s+=i*i}')"
