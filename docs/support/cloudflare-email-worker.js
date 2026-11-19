// Cloudflare Email Worker for an Orbit support mailbox (docs/support.md).
// Email Routing sends each email for the support address to this Worker, which POSTs the raw
// message to the mailbox's inbound URL. Store the URL as a secret, not in this file:
//   npx wrangler secret put ORBIT_INBOUND_URL
export default {
  async email(message, env) {
    // The raw stream can be read only once: keep the bytes for one retry.
    const raw = await new Response(message.raw).arrayBuffer()
    for (let attempt = 1; attempt <= 2; attempt++) {
      const response = await fetch(env.ORBIT_INBOUND_URL, {
        method: 'POST',
        headers: { 'Content-Type': 'message/rfc822' },
        body: raw,
      }).catch(() => null)
      if (response?.ok) return
      // 4xx: Orbit refused this email (wrong URL, too large); a retry gives the same answer.
      if (response && response.status < 500 && response.status !== 429) {
        return message.setReject(`Support mailbox refused the message (${response.status})`)
      }
    }
    message.setReject('Support mailbox is not reachable, try again later')
  },
}
