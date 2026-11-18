# Option 1: a built-in helpdesk in Orbit (design, 2026-11-18)

Status: phase 1 (MVP) built on branch `helpdesk-mvp` (not deployed). What shipped and how it works: `docs/support.md`
and `.ai/FEATURES.md` → Support email. Phases 2–4 below are still open. Model: Zammad, FreeScout, Chatwoot (shared mailbox,
conversation = ticket, reply vs. internal note, status open/pending/solved).
Base: Orbit `main` (c0b8f0e5) in /home/exedev/orbit.

## 1. Main idea: a ticket is a task in a "Support" project

Orbit already has most helpdesk parts in Tasks. We reuse them and do not build a second inbox page.

| Helpdesk part | Orbit part that we reuse |
|---|---|
| Shared inbox | A project with a connected support mailbox. List/Board views, filters, saved views, bulk bar |
| Ticket | A task (`tasks`), title = email subject |
| Status open / pending / solved | The project's workflow statuses (`.ai/FEATURES.md:159`): Open, Pending, Solved, Closed |
| Assignee, auto-assign | Assignees + label rules / owner map (`.ai/FEATURES.md:91`) |
| Internal notes | Normal task comments (rich text, @mentions, files) |
| Watchers, alerts | Followers + Inbox notifications + iPhone push (`apps/server/src/push/apns.rs`) |
| Who can see | Project access: Workspace or Private (`.ai/FEATURES.md:166`). Guests never see support projects |
| Work items from a ticket | Sub-issues and task relations (bug for the dev team, linked to the ticket) |

New parts (small):
- `support_mailboxes` (workspace, project, address, display name, inbound secret hash, outbound SMTP settings encrypted
  with the app key like the SMTP password today).
- `support_contacts` (workspace, email, name). Customers are NOT Orbit users and never sign in.
- `support_messages` (task, direction in/out, from/to/cc, subject, Message-ID, In-Reply-To, References, sanitized
  HTML, plain text, attachment ids, send state). The task timeline shows them between the comments.
- The task composer gets two tabs: **Reply to customer** (sends email) and **Internal note** (a comment, default).
  The reply tab has a clear colour and shows "Sends email to x@y.com". This is the main guard against leaking notes.
- The mock **Mail** page (`.ai/FEATURES.md:522`, seed data today) is removed. Support lives in Tasks only:
  one place per feature. Contacts: a small "Customer" card in the task side panel (email, other tickets of this contact).

Core flow:
1. Email to support@ → inbound endpoint → parse → find the thread by `In-Reply-To`/`References` (our own Message-IDs
   carry the ticket id + an HMAC), then by a `[#ORB-123]` subject tag. No match → new task in status Open.
2. New task: label rules assign it (for example label `billing` → Anna). Followers get push.
3. Agent answers with **Reply to customer** → outbound job sends the email (From = the support address,
   threading headers set) → status moves to Pending (setting per mailbox).
4. Customer answers → message added, task back to Open, assignees notified.
5. Agent sets Solved. Solved tickets that get a new customer email reopen; Closed ones (after N days) make a new ticket.

## 2. Donna

Donna works on tickets with the tools she has for tasks (`get_task`, comments, labels), plus three new ones:
- `summarize_ticket` (read only): short summary of the thread for the side panel or Chat.
- `suggest_triage`: proposes labels and assignee. It writes them only as the driver, like any task edit.
- `draft_reply`: puts a draft into the Reply tab of the driver. **Donna never sends email herself in the MVP.**
  A person presses Send. Later, if wanted: `send_ticket_reply` behind the existing driver approval card
  (`.ai/agent-design.md:555`, approval bound to driver, thread and the hash of the exact text and recipients).
- Prompt injection: customer email is the most hostile input Orbit will have. It goes to Donna only as fenced
  untrusted data (`.ai/agent-design.md:115`). Rules for this: Donna never takes recipients or links from the email
  body, and no auto-run of Donna on new tickets in the MVP (a routine can do triage later, still without sending).

## 3. Email in and out

**In (recommended): one inbound endpoint that takes raw MIME**, `POST /api/v1/hooks/support/<mailbox_id>` with a secret
URL (hash stored, rotate like chat webhooks, `docs/chat-webhooks.md`). It goes through the existing public hooks gateway
(`ORBIT_PUBLIC_HOOKS_URL`, orbit-hooks.easytomanage.com). Orbit parses the MIME in Rust with the `mail-parser` crate
(Stalwart, safe Rust, no deps). Two ways to feed it:
- **Cloudflare Email Routing + a small Email Worker** that POSTs the raw message (free, if the support domain is on
  Cloudflare). The Worker is ~20 lines; we ship it as a copy-paste file in the docs.
- **Postmark inbound** (JSON with base64 attachments, `StrippedTextReply`; it waits 2 min for our 200). Paid. We take
  its `RawEmail` field (when enabled) so one parser path stays.

**IMAP poll (phase 3)** for teams that keep Gmail/Microsoft 365 mailboxes (FreeScout's way). More work: a poll job per
mailbox, UID state, OAuth for M365/Gmail (basic auth is off there). Plain forwarding from Gmail to the Cloudflare
address works in the MVP and covers most of this.

**Out:** the jobs queue sends with `lettre` (already in `apps/server/Cargo.toml:27`) through the mailbox's own SMTP
(for example the provider of the support domain), so SPF/DKIM stay correct for that domain. Not the instance SMTP:
on Orbit Cloud that would send customer mail of many workspaces from one domain. Retry with backoff; failed sends
show "Not sent, retry" on the message.

Server needs: no new service and no open port 25. One public POST route behind the gateway, the jobs queue,
storage for attachments (S3 when on), a size limit (25 MB per email), and a send cap per mailbox (`email_sends`).

## 4. Phases and size

| Phase | Content | Size (one build session) |
|---|---|---|
| 1 MVP | Mailbox setup in Project settings, inbound endpoint + MIME parse + threading, contacts, messages on the task timeline, Reply/Note tabs, outbound SMTP job, Open/Pending/Solved auto-moves, sanitized HTML view, attachments, push, docs + Cloudflare Worker file | L: ~1.5 to 2 weeks |
| 2 Donna + iPhone | `summarize_ticket`, `suggest_triage`, `draft_reply`; iPhone: messages in task detail + reply tab | M: ~1 week |
| 3 Comfort | Canned replies (Docs pages as templates), auto-reply on new ticket, auto-close after N days, contact page, signatures | M: ~1 week |
| 4 More channels | IMAP poll with OAuth, website chat widget (Chatwoot-like), CSAT, SLA timers, reports | L each, only on demand |

## 5. Risks and security

- **Tenant isolation:** a mailbox belongs to one workspace and project; the inbound route finds the mailbox by its id +
  secret only, never by the To address alone (a spoofed To must not reach another workspace).
- **IDOR:** ticket = task, so all reads go through the existing task and project access checks. New routes
  (`/support-mailboxes/{id}`, message attachments) check workspace + project access for each id. Thread match ids in
  Message-IDs carry an HMAC, so a customer cannot attach mail to another ticket by guessing ids.
- **XSS from email HTML:** clean on the server with `ammonia` (allow list; no script, style, forms, iframes, event
  attributes; `cid:` images rewritten to our attachment URLs), then show it in a sandboxed iframe with a strict CSP.
  External images blocked by default ("Show images"), so tracking pixels do not fire.
- **Attachments:** size and count caps, `Content-Disposition: attachment`, no inline render of HTML/SVG, the same
  storage and type checks as task attachments.
- **Spoofing and spam:** read SPF/DKIM/DMARC results that the provider adds; mark failed mail "Unverified sender".
  Rate limit per mailbox; drop auto-replies and bounces (`Auto-Submitted`, `Precedence: bulk`) to stop mail loops.
- **Leak of notes:** the Reply/Note split above; replies show recipients before send.
- **Secrets:** SMTP passwords encrypted with the app key; inbound secrets stored as hashes; nothing in logs.
- **Backups and migrations:** new tables only, no change to existing rows; DB backup before deploy as usual.
- **Conflicts:** touches task detail (web + iPhone), comments timeline and Donna tools. Must merge after or rebase on
  open branches in `apps/web/src/features/tasks` and `apps/agent`.
- **Machine:** low load; email bodies and attachments add disk use (S3 helps).

## 6. Not in the MVP

Live chat widget, customer portal / sign-in, knowledge base, SLA timers, CSAT surveys, reports, IMAP polling,
multi-brand per mailbox, Donna sending email by herself, Donna auto-run on new tickets, merge/split tickets,
macros/canned replies, other channels (WhatsApp, social).

## Sources

- Postmark inbound webhook: https://postmarkapp.com/developer/webhooks/inbound-webhook ,
  https://postmarkapp.com/developer/user-guide/inbound/parse-an-email
- Cloudflare Email Workers handler: https://developers.cloudflare.com/email-service/api/route-emails/email-handler/
- mail-parser crate: https://docs.rs/mail-parser
- ammonia crate: https://docs.rs/ammonia
- FreeScout fetching emails (IMAP model): https://github.com/freescout-help-desk/freescout/wiki/Fetching-Emails
