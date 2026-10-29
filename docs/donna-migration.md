# Donna → Orbit migration

Assessment date: October 29, 2026. Status: planning only; Donna has not been inspected,
modified, stopped, or migrated. Do not delete `donna-wellrox` on the strength of this document.

## Goal and current boundary

Retire Hermes and its VM while preserving Wellrox's knowledge, active work, history,
and required automations. Orbit should become the team's system of record.
Running Hermes on the Orbit VM would consolidate hosting, not accomplish this migration.

Local evidence: `docs/agent.md`, `apps/agent/src/tools.ts`, and `DEPLOYMENT.local.md`.
During this assessment, Orbit, nginx, and the agent services were active; Orbit readiness
passed and the sidecar reported `mock: false`. These are infrastructure checks, not an
end-to-end acceptance test of a signed-in agent run.

Orbit's agent can work with tasks, Docs, chat, and team/private memory. It acts under a
human driver's permissions in one workspace. It has no shell, filesystem, browser,
external-service tools, delete tool, or general scheduled-agent tool. A healthy platform
scheduler does not mean Donna's scheduled prompts can run on the Orbit agent.
There is no Hermes importer in the reviewed agent tool set.

## 1. Read-only inventory

Obtain approved SSH access to Donna or a protected export from its owner. Do not paste
credentials into Orbit chat, Docs, this repository, or the migration report.

- [ ] Record installed Hermes version, service users, active profiles and actual
  `HERMES_HOME` locations; do not assume everything is in `~/.hermes`.
- [ ] Inventory identity/instructions, memories, skills, plugins, session databases,
  transcripts, files, external memory stores, repositories, and uncommitted work.
- [ ] Inventory Hermes cron jobs, OS crontabs, systemd timers, containers, webhooks,
  messaging gateways, and other processes on the VM.
- [ ] List integrations, credential *names* and owners, OAuth grants, scopes, callback
  addresses, attached exe.dev integrations, domains, and network dependencies.
- [ ] For each responsibility record its trigger, inputs, outputs, timezone, last success,
  next due run, business owner, required access, failure alert, and acceptance test.
- [ ] Review recent actual activity alongside Donna's own handover summary. Neither the
  summary nor a folder listing alone establishes that the inventory is complete.
- [ ] Classify each item: migrate directly / build replacement / intentionally retire.
  Every required item needs an owner and destination.

## 2. Preserve before transforming

- [ ] Use the installed version's supported backup procedure after inspecting its help.
  Current Hermes documentation describes `hermes backup`, but support must be checked on Donna.
- [ ] Protect the archive as a secret: Hermes backups may contain credentials and private
  conversations. Encrypt it and keep a verified off-VM copy.
- [ ] Include external working directories, memory stores, service definitions, and other
  dependencies not covered by the Hermes archive. Record hashes and counts.
- [ ] Verify archive integrity and rehearse recovery in isolation with outgoing messages,
  webhooks, scheduled actions, and other side effects disabled.
- [ ] Retain an untouched source archive and a separately sanitized migration dataset.
  Use database-consistent backup methods, not a raw copy of a live SQLite main file.
- [ ] Back up the Orbit destination before importing. Protect its existing app key and
  separately back up the sidecar's `pi.sqlite`; Orbit backups do not include sidecar data.

## 3. Map information and capabilities

| Source | Proposed destination / constraint |
| --- | --- |
| Business knowledge, research, SOPs, decisions | Structured Orbit Docs, with source references and review dates |
| Short durable team facts | Reviewed team memory, backed by source Docs or an import handover thread |
| Personal preferences or confidential context | Only the correct person's private space/memory after explicit review; never bulk-copy to team memory |
| Open work, commitments, follow-ups | Orbit tasks with owners, status, due dates, dependencies, and source IDs |
| Historical conversations | Protected original archive plus reviewed summaries; do not fabricate native Orbit chat history |
| Attachments and generated deliverables | Supported Orbit file surfaces, preserving names, provenance, and confidentiality |
| Persona, skills, operating instructions | Reviewed SOPs; implement approved behavioral requirements explicitly where needed. Memory is data, not an instruction loader |
| Browser, shell, research, external APIs | Missing from the current agent; implement narrowly scoped replacements or explicitly retire the responsibility |
| Scheduled prompts and proactive messages | Missing from the current agent; design durable triggers, execution identity, delivery, retries, deduplication, and alerts |
| External messaging / email gateways | Audit actual use and replace only required channels; do not assume Orbit chat or mock Mail is an equivalent gateway |
| Credentials | Reauthorize supported connections with least privilege; do not import raw tokens as content |

For unattended work, decide who owns and authorizes execution and model usage. Do not
silently reuse a person's interactive session. Prefer narrow auditable integrations over
granting the Orbit agent unrestricted host access.

Build replacement capabilities before cutover if Donna's inventory proves they are needed.
Any application changes must follow repository security, database, testing, and web/mobile
parity requirements. This planning document changes neither app.

## 4. Controlled import and acceptance

- [ ] Confirm the target Wellrox workspace, membership, and privacy boundaries.
- [ ] Build a manifest mapping source IDs/paths and hashes to destination IDs, with counts,
  errors, exclusions, and reviewer approval. Make import reruns idempotent.
- [ ] Use supported authenticated application paths, not direct database inserts.
  Build a dedicated importer only if volume warrants it; do not bypass agent route restrictions.
- [ ] Treat imported content as untrusted data. Exclude secrets and review instructions,
  personal information, stale facts, and third-party confidential information.
- [ ] Verify document formatting, files, links, task ownership, dates/timezones, and search.
- [ ] Test representative real Donna requests end-to-end: fact recall with a source, SOP
  retrieval, task update, handover, and every required integration or recurring workflow.
- [ ] Test private-context isolation, denied permissions, restart recovery, provider failure,
  alert delivery, and duplicate-action prevention.
- [ ] Assign a human approver to each required responsibility. Log gaps rather than assuming
  that matching a few conversational answers proves parity.

## 5. Shadow, cut over, then retire

- [ ] Run a shadow period covering every business-critical recurring workflow at least once.
  Seven to fourteen days is a proposed window, not proof that monthly jobs are covered;
  test infrequent jobs safely as well.
- [ ] Keep only one system authorized to send messages or perform each external write.
  Shadow runs should be read-only or use isolated test destinations.
- [ ] Agree on cutover time, owner, rollback window, rollback triggers, and communication.
- [ ] Pause Donna's writers and capture the final consistent backup/delta. Finish the import,
  reconcile queued work, then enable the replacement writers.
- [ ] Stop/disable Hermes and other obsolete services without deleting the VM yet.
  Verify Orbit workflows work with Donna unavailable; monitor errors and missed schedules.
- [ ] Keep Donna recoverable for an agreed period. Rollback must pause replacement writers
  first and reconcile actions already taken; blindly restoring a snapshot can lose new work.
- [ ] Require explicit owner sign-off before VM deletion. Verify the protected off-VM archive
  remains accessible and that no required files, endpoints, callbacks, or workloads depend on Donna.
- [ ] Revoke obsolete grants, rotate migrated/shared secrets as appropriate, remove stale
  integrations/DNS/webhooks, and delete the VM only after approval.

## References

Hermes's current documentation is discovery guidance, not evidence of Donna's installed layout:

- https://hermes-agent.nousresearch.com/docs/user-guide/configuration
- https://hermes-agent.nousresearch.com/docs/reference/cli-commands
- https://hermes-agent.nousresearch.com/docs/reference/faq

Next action: read-only Donna audit, yielding a redacted inventory and a concrete
capability-gap list before estimating implementation or scheduling retirement.
