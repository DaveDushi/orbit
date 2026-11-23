# Support email (helpdesk)

A project can have a **support mailbox**. Then customer email becomes tasks in that project: one
task is one ticket (one conversation). Members answer from the task, with the same tools as for
any task: assignees, labels and label rules, followers, the Inbox, sub-issues and relations.
There is no separate inbox page: support lives in Tasks. Design: `.ai/helpdesk-design.md`.

## Connect a mailbox

Owners and admins open the project, then **Project settings → Support email**:

| Field | Meaning |
|---|---|
| Support address | The address customers write to, for example `support@example.com`. Replies come from it. |
| Sender name | The name customers see on replies, for example `Acme Support`. |
| SMTP server, port, security, user name, password | The mail server that sends the replies: the one of your support domain, so SPF and DKIM stay correct. Without one, the project only receives email. The password is stored encrypted with the server's app key (`ORBIT_APP_KEY`). Only public hosts and the ports 25, 465, 587 and 2525 are allowed; security `none` and local hosts only in development. |
| Move to Pending after a reply | A reply moves the ticket to the Pending status (on by default). |
| Assign new tickets to | A member who gets each new ticket. |

**Connect mailbox** adds the statuses **Open** (Unstarted), **Pending** (Started), **Solved**
(Completed) and **Closed** (Cancelled) to the project when it does not have them (by name), and
shows the **inbound URL** once. Copy it: Orbit stores only a hash of its secret. **Make new URL**
gives the mailbox a new secret; the old URL stops working at once. **Disconnect** stops the URL;
the tickets and their emails stay, but they can no longer be answered.

Support mailboxes need `ORBIT_APP_KEY` (for the SMTP password and the signed Message-IDs).

## Send email to Orbit

Orbit has no mail server of its own. A mail provider receives the email and posts the raw message
(RFC 5322, as received) to the inbound URL:

```bash
curl -X POST -H 'Content-Type: message/rfc822' --data-binary @email.eml \
  'https://orbit.example.com/api/v1/hooks/support/<mailbox_id>/<secret>'
```

Answers: `200 ok` (filed, or the same Message-ID came before), `200 dropped` (an auto-reply or
bounce, see below), `404` (unknown mailbox or wrong secret), `413` (more than 36 MB), `422` (not an
email, or no valid `From`), `429` (more than 60 emails in a minute for this mailbox), `402` (the
workspace is read-only on Orbit Cloud).

The URL uses the hooks origin (`ORBIT_PUBLIC_HOOKS_URL`, else `ORBIT_PUBLIC_ORIGIN`). When the hooks
origin is a gateway in front of a login-protected Orbit, the gateway must also let through
`POST /api/v1/hooks/support/…` (see [operations](operations.md)).

### Cloudflare Email Routing

If the support domain uses Cloudflare:

1. Create a Worker from [`support/cloudflare-email-worker.js`](support/cloudflare-email-worker.js)
   (about 20 lines) and store the inbound URL as its secret:
   `npx wrangler secret put ORBIT_INBOUND_URL`.
2. In **Email → Email Routing → Routing rules**, send the support address to the Worker.

The Worker posts each email as it arrived and retries once on a network error or a `5xx`/`429`.
When Orbit refuses an email, the Worker rejects it, so the sender gets a bounce. Cloudflare's
handler reference: <https://developers.cloudflare.com/email-service/api/route-emails/email-handler/>.

Other providers work too when they can post the raw message (for example Postmark inbound with
the raw email enabled). A Gmail or Microsoft 365 mailbox can forward to the Cloudflare address.

### Microsoft 365 (an existing mailbox)

A team that already has `support@` in Microsoft 365 (Exchange Online) keeps it. Microsoft 365
cannot post to a URL, so its mail goes through the Cloudflare Worker above:

1. Set up the Worker with the mailbox's inbound URL on an address of a domain (or subdomain) on
   Cloudflare, for example `support-inbound@help.example.com`.
2. **Exchange admin center → Mail flow → Rules → Add a rule**: *Apply this rule if* the recipient
   is `support@example.com`, *Do the following* **Redirect the message to**
   `support-inbound@help.example.com`. A redirect keeps the customer in `From`. The mailbox keeps
   its copy only if you also choose "Deliver the message to … and redirect" (a BCC action); this
   is optional.
3. An **Outlook inbox rule** ("Forward to" or "Redirect to") on the mailbox works too, but needs
   external forwarding on: **Microsoft Defender → Email & collaboration → Policies → Anti-spam →
   Anti-spam outbound policy → Automatic forwarding rules → On** (for the organization, or a policy
   for this mailbox). A mail flow rule (step 2) does not need it.
4. Replies: in Project settings → Support email, SMTP server `smtp.office365.com`, port `587`,
   STARTTLS (the defaults of a new mailbox), user name empty or the mailbox address, and the
   mailbox's password. Turn on **Authenticated SMTP** for the mailbox: **Microsoft 365 admin center
   → Users → Active users → the mailbox → Mail → Manage email apps → Authenticated SMTP**. If the
   tenant turns SMTP AUTH off for everyone, also clear **Exchange admin center → Settings → Mail
   flow → Turn off SMTP AUTH protocol for your organization**. Changes can take up to an hour.
   Then select **Test connection**: it signs in and quits without sending.

The mailbox settings show these steps under **Use an existing Microsoft 365 mailbox**.

Errors the test and failed replies explain:

| Answer | Meaning |
|---|---|
| `535 5.7.139 … SmtpClientAuthentication is disabled for the Mailbox` (or `… for the Tenant`) | Authenticated SMTP is off: step 4. |
| `535 5.7.3` / other `535` | Wrong user name or password. The user name is the full mailbox address. |
| `550 5.7.30 Basic authentication is not supported for Client Submission` | The tenant no longer allows a password for SMTP. |

Microsoft retires password sign-in (Basic authentication) for SMTP AUTH: per Message Center
MC786329 (updated January 2026) it is off by default for existing tenants from the end of
December 2026, an admin can allow it again for a while, and the final date comes in 2027
(<https://mc.merill.net/message/MC786329>). Orbit signs in with a password only (no OAuth yet);
after that date use another SMTP server for replies (for example the provider of the support
domain, or a relay such as SMTP2GO or Postmark) with the same From address, and keep the redirect
of step 2 for incoming mail.

**Forwarded mail keeps the customer.** When the mailbox itself or a colleague of the support domain
forwards an email (Outlook "Forward"), `From` is that mailbox. Orbit then takes the customer from
the forwarded block (`From:` under `-----Original Message-----`, Outlook's line, Gmail's
"Forwarded message"; also `Von:`, `De:`, `Van:`), or from `Reply-To`, when that address is outside
the support domain **and** the email is proven to come from the support domain: the topmost
`Authentication-Results` is the one of Cloudflare Email Routing (`mx.cloudflare.net`) and says
`dmarc=pass header.from=<support domain>` or `dkim=pass header.d=<support domain>`, and no
`Authentication-Results` of an earlier hop says it failed. Anyone can write a support-domain
`From`, so without this proof `From` stays the requester and the email is marked **Unverified
sender** (a forged forward cannot choose who gets the reply). A redirect keeps `From` as it is. A forward with no customer in it stays a
ticket of the support address, and a reply to it is refused (it would loop).

## Tickets

An inbound email finds its ticket in this order, only among the tickets of the same mailbox:

1. `In-Reply-To` / `References` name a reply Orbit sent. Orbit's Message-IDs carry the task id and
   an HMAC of it under the app key, so a made-up id cannot put mail into another ticket.
2. `In-Reply-To` / `References` name an earlier email of the ticket.
3. The subject has the ticket's tag, for example `[#SUP-12]`, and the sender is the ticket's
   customer.

A match in a Closed (Cancelled) ticket or a trashed task is no match. Without a match, the email
makes a new task in **Open**, with the subject as its title, created by the service account
"Support email". A customer email to a **Solved** (Completed) or **Pending** ticket moves it back
to **Open**. Redeliveries (same Message-ID) are filed once.

The task's activity shows the emails between the comments. HTML email is cleaned on the server
(allow list: no scripts, styles, forms, frames or event attributes; links open in a new tab) and
shown in a sandboxed frame with its own Content Security Policy. Images from other sites are
blocked until the reader selects **Show images**, so tracking pixels do not fire. Images inside the
email (`cid:`) show from the ticket's attachments.

Attachments become task attachments of the ticket (the same storage, quota and download rules:
only raster images show inline, everything else downloads). An email keeps at most 20
attachments and 25 MB of them; the rest is left out, and the email says how many.

**Unverified sender**: the email's `Authentication-Results` (added by the receiving provider) say
DMARC failed, or SPF or DKIM failed without a DKIM pass, or a forward from the support domain
names another customer without proof (see above). Treat such mail as possibly forged.

**Dropped**: auto-replies and bounces are not filed, so two auto-responders cannot loop:
`Auto-Submitted` other than `no`, `Precedence: bulk/junk/list`, `X-Autoreply`, `X-Autorespond`,
a null return path, mail from `MAILER-DAEMON` or `postmaster`, and delivery reports.

The side panel of a ticket shows the **Customer** (name, address) and their other tickets that you
can see. Customers are not Orbit users and never sign in.

## Reply

The composer of a ticket has two tabs:

- **Internal note** (the default): a normal comment. It is never emailed.
- **Reply to customer**: a blue field that says **Sends email to** the customer's address. The
  reply is plain text, from the mailbox address, with `Re: <subject> [#KEY-12]` and the threading
  headers. **Send email** (or Cmd/Ctrl+Enter) queues it; a background job sends it.

A reply shows *Sending…*, then *Sent*. Network errors and temporary SMTP errors retry 4 times
(30 s, 2 min, 10 min, 30 min). After that, or on a permanent SMTP error, the reply shows
**Not sent** with **Retry**. A mailbox sends at most 500 replies in 24 hours.

Assignees and followers of a ticket get an Inbox notification (and a push, as for followed tasks)
for each customer email.

## Access

- Mailbox setup: owners and admins (`integrations.manage`).
- Tickets are tasks: who sees the project sees its tickets, their emails and the customer, and can
  reply. Private projects limit this to their members. Guests never see support projects.
- The inbound route finds the mailbox by its id and secret only, never by the address the email
  was sent to.
- Nothing secret is logged: not the URL secret, not the SMTP password, not message bodies.

## API

| Route | |
|---|---|
| `GET/POST/PUT/DELETE /api/v1/workspaces/{id}/projects/{project_id}/support-mailbox` | The mailbox (owners and admins). |
| `POST …/support-mailbox/rotate` | A new inbound URL. |
| `POST …/support-mailbox/test` | Signs in to the SMTP server and quits (`{ ok, message }`); never sends. |
| `GET /api/v1/workspaces/{id}/tasks/{task_id}/support` | The task's emails, customer and other tickets. |
| `POST /api/v1/workspaces/{id}/tasks/{task_id}/support/replies` | Queues a reply (`{ "body": "…" }`). |
| `POST /api/v1/workspaces/{id}/support/messages/{message_id}/retry` | Sends a failed reply again. |
| `POST /api/v1/hooks/support/{mailbox_id}/{secret}` | Inbound email (raw MIME body). |

Not yet: OAuth sign-in for Microsoft 365 SMTP, Donna tools for tickets, tickets in the iPhone app, IMAP polling, canned replies,
signatures, auto-close, attachments on replies.
