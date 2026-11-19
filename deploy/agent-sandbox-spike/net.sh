P=${HTTPS_PROXY#http://}; AUTH=${P%@*}; HP=${P#*@}; PH=${HP%:*}
echo "proxy=$PH"
echo "--- allowed host via proxy:"; printf 'CONNECT allowed.test:443 HTTP/1.1\r\nProxy-Authorization: Basic x\r\n\r\n' | timeout 5 nc -w 3 $PH 3128
echo "--- unknown host via proxy:"; printf 'CONNECT evil.example:443 HTTP/1.1\r\n\r\n' | timeout 5 nc -w 3 $PH 3128; echo
echo "--- plain GET via proxy:"; printf 'GET http://evil.example/ HTTP/1.1\r\n\r\n' | timeout 5 nc -w 3 $PH 3128
echo "--- other host ports (ssh 22, orbit 8000):"; for port in 22 8000 53; do timeout 5 nc -w 2 $PH $port </dev/null; echo "port $port exit=$?"; done
echo "--- direct internet / metadata / DNS:"; timeout 5 nc -w 2 1.1.1.1 443 </dev/null; echo "1.1.1.1 exit=$?"; timeout 5 wget -q -T 3 -O- http://169.254.169.254/ 2>&1 | head -1; timeout 5 nslookup example.com 2>&1 | tail -1
echo "--- other sandbox /30 (10.200.2.1) and docker bridge 172.17.0.1:"; timeout 5 nc -w 2 10.200.2.1 3128 </dev/null; echo "exit=$?"; timeout 5 nc -w 2 172.17.0.1 80 </dev/null; echo "exit=$?"
echo "--- add a route as uid 1000:"; ip route add default via $PH 2>&1 | head -1
