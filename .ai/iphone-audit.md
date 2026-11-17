# iPhone app parity audit (2026-11-17)

Page-by-page check of the iPhone app (`apps/mobile`) against the web app (`apps/web`), branch `iphone-full-audit`.

## How it was checked

- **Inventory:** every route of `apps/web/src/app/App.tsx`, the areas of `settingsNav.ts`, the dialogs and menus of each
  feature, and every API call the web makes that the iPhone does not (`sdk.gen.ts` functions used by `apps/web` only).
- **Running app, three roles:** a test instance like production: the team agent ON (sidecar with `AGENT_MOCK=1`, mock
  ChatGPT sign-in), `ORBIT_PUBLIC_MCP_URL` set (test value `https://orbit-mcp.ifa-test.invalid`), Drive on a local S3
  test store (RustFS, test keys), Mailpit, seed data in every area (projects, tasks with sub-issues, relations,
  milestone, cycle, comments; views; teamspace, pages, comment thread, trash; channels, thread, DMs, guest channel;
  Drive folder and files; agents, a skill, team instructions, a routine made through the agent). Roles: admin
  (`test@example.com`, root), member (`member@example.com`), guest (`guest@example.com`, one private channel), strong
  random passwords. The iPhone app ran as its Expo web build at 390×844 behind a proxy that drops `Origin`, so it
  signs in with a bearer token like iOS; Playwright opened every screen for each role and clicked through each fix.
- **Not testable here:** GitHub (needs a real GitHub App; no production secrets were copied), APNs push, Apple speech.
  The web build of the app has no live sockets (a browser WebSocket cannot send the bearer header), so live updates
  were checked in code and by unit tests (`data/live.test.ts`).

Status: **OK** = same as the web (or the phone's own pattern for it); **missing** = not on the iPhone; **broken** =
there but wrong; **hidden** = there but hard to reach, or hidden by a condition without a message; **n/a** = no phone
counterpart on purpose. "Fixed" in the fix column means fixed in this branch; screenshots are in
`~/.local/share/com.jean.desktop/pasted-images/iphone-full-audit/<row>-*.png`.

## Shell and global

| Row | Route | Web has | iPhone has | Status | Fix |
|---|---|---|---|---|---|
| G01 | sidebar | Home, Tasks, Docs, Chat, Drive, Activity, Settings, Admin, Apps | Dock (Home, Tasks, Docs, Chat, Drive, Settings), bell in every tab header, ☰ drawer | OK | — |
| G02 | workspace switcher | Switch, Create workspace, Join | Only in the ☰ drawer | hidden | Fixed: one `WorkspaceSwitcher`, also a "Switch workspace" row at the top of Settings (every role) |
| G03 | ⌘K search | Tasks, docs, chats, Drive | Search screen (Home, Settings, drawer) | OK | — |
| G04 | Ask bar | Ask the agent (ChatGPT connected, not guests) | Ask bar above the dock, same rule | OK | — |
| G05 | account menu | Status, custom status, appearance, sign out | Account menu (drawer), Settings | OK | — |
| G06 | read-only / mail banners | Billing and mail-server banners | Same banners above the dock | OK | — |
| G07 | live updates | Every workspace event refreshes the workspace | Saved views (views-only events were dropped), Home layout, apps, agents/skills/agent tools, memory, exports and stickers did not refresh | broken | Fixed: `workspaceDataKeys` covers them, views-only events refresh views, `stickers.changed` handled; regression tests in `data/live.test.ts` |
| G08 | profile popover | Tap a person: title, local time, email, phone, role, bio, Message, private note | Nothing | missing | Fixed: `ProfileSheet` from a chat author and a channel member (C13) |
| G09 | `/profile/shortcuts` | Keyboard shortcuts | — | n/a | No keyboard on a phone |
| G10 | `/profile/storage` | This browser's chat cache | — | n/a | The app manages its own cache |

## Sign-in and public pages

| Row | Route | Web has | iPhone has | Status | Fix |
|---|---|---|---|---|---|
| P01 | `/login` | Password, two-factor, passkey | Password, two-factor | missing (passkey) | Open: native passkey sign-in needs an iOS associated domain and WebAuthn in the app |
| P02 | `/register`, `/recovery` | Forms | Open the web pages | OK | By design: the server refuses anonymous native writes (lessons.md) |
| P03 | `/accept-invitation` | Accept, new account | Join with an invitation link (signed in); a new account uses the web | OK | — |
| P04 | `/confirm-email`, `/setup`, `/oauth/consent`, `/s/:token` | Browser pages | Links open the browser | n/a | — |
| P05 | `/onboarding` | No workspace: create or join | `NoWorkspace` (create, join) | OK | — |

## Home and Activity

| Row | Route | Web has | iPhone has | Status | Fix |
|---|---|---|---|---|---|
| H01 | `/` | Cards; Customize: add, rows, move, remove, drag | Same cards; pencil: add, rows, up/down, remove | OK | Drag is up/down on the phone |
| H02 | `/` guest | Chat cards only | Chat cards only | OK | — |
| V01 | `/activity` | All/Unread/Mentions, mark read, mark all read, open target | Same | OK | — |

## Tasks

| Row | Route | Web has | iPhone has | Status | Fix |
|---|---|---|---|---|---|
| T01 | `/tasks` | List, board, filters, display, search, presets, project picker | Same (Views and Trash in the view menu) | OK | — |
| T02 | `/tasks` | New project (sidebar) | Only in the ☰ drawer | hidden | Fixed: "New project" at the end of the project menu |
| T03 | `/tasks/:id` | Edit title and description (co-edited) | Both read-only | missing | Fixed: title field; description in the collab editor (task schema), toolbar, inline images; GitHub-synced and read-only Cloud workspaces stay read-only |
| T04 | `/tasks/:id` | Comments: reply in thread, edit, delete | Threads shown, no reply/edit/delete | missing | Fixed: options button per comment; composer replies or edits; delete asks first |
| T05 | `/tasks/:id` | Move to project | Project read-only | missing | Fixed: Project chip → project list (bulk write like the web) |
| T06 | `/tasks/:id` | Repeat (recurrence) | Nothing | missing | Fixed: Repeat sheet (interval, frequency, weekdays, base) |
| T07 | `/tasks/:id` | Status, priority, assignees, labels, due, milestone, cycle, estimate, parent, source, followers | Same | OK | — |
| T08 | `/tasks/:id` | Sub-issues, relations, linked docs, duplicate of, attachments | Same | OK | — |
| T09 | `/tasks/:id` | GitHub pull requests of the task, "read-only while linked" | Nothing | missing | Fixed: pull request list and the read-only note |
| T10 | `/tasks/:id` | Context menu: copy ID/title/link, labels, due, cycle, milestone, estimate, move | Long press: status, priority, assign to me, done, trash | OK | The rest is on the task screen |
| T11 | `/tasks/projects/:id/settings/rules` | Label rules, owner:<name> switch and map | Link to the web | missing | Fixed: native Rules card (owners/admins; others get a message) |
| T12 | `/tasks/projects/:id/settings/github` | Connect a repository | Link to the web (in-app browser) | missing | Open: GitHub's install pages are web; not testable without a GitHub App |
| T13 | `/tasks/projects/:id/settings/*` | General, statuses, access, labels, milestones, cycles, docs, sub-issues, danger zone | Same | OK | — |
| T14 | `/tasks-trash` | Search, restore, delete forever, empty | Same | OK | — |
| T15 | `/tasks-trash/task/:id`, `/project/:id` | Read-only preview | Row menu only | missing | Open (low) |
| T16 | `/views`, `/views/:id` | Favorite, create, edit, duplicate, delete, save changes | Same | OK | — |
| T17 | `/views` | Reorder favorites; per-view display preference | — | missing | Open (low) |
| T18 | `/tasks` | Multi-select bulk toolbar | — | missing | Open: a desktop pattern; per-task long press instead |
| T19 | `/tasks` | Drag to reorder tasks and projects | — | missing | Open (FEATURES.md already lists project order as not ported) |
| T20 | `/tasks` | Timeline layout | A timeline view shows as a list | missing | Open |

## Docs

| Row | Route | Web has | iPhone has | Status | Fix |
|---|---|---|---|---|---|
| D01 | `/docs` | Favorites, teamspaces, private, new page/sub-page, teamspace menu | Same | OK | — |
| D02 | `/docs/:id` | Co-editing, toolbar, images, icon, cover, title | Same | OK | — |
| D03 | `/docs/:id` menu | Favorite, move, lock, history, duplicate, export, trash, print | Same but print | OK | Print is n/a |
| D04 | `/docs/trash` | Restore, delete forever, empty, preview | Restore, delete forever, empty | missing (preview) | Open (low) |
| D05 | `/docs/import` | Import from Notion | Same | OK | — |
| D06 | `/docs/:id` comments | Resolve, reopen, delete thread, edit and delete own comment | List and reply only | missing | Fixed: Resolve/Reopen, Delete thread (creator), Edit/Delete own comment; rules tested |
| D07 | `/docs/:id` Drive file block | File name, open | "Drive file — Open on the web" (written before the app had Drive) | broken | Fixed: name and access from the server, tap opens the file |
| D08 | `/docs/:id` | Comment on a selected passage | Page comments only | missing | Open: needs text selection in the editor web view |
| D09 | `/docs/:id` | Page file blocks (CSV/XLSX edit, versions) | — | missing | Open |
| D10 | `/docs` | Reorder favorites, recent pages | — | missing | Open (low) |

## Chat

| Row | Route | Web has | iPhone has | Status | Fix |
|---|---|---|---|---|---|
| C01 | `/chat` | Channels, DMs, categories, unread, star | Same | OK | — |
| C02 | `/chat/unreads`, `/chat/threads` | Lists, mark read | Same | OK | — |
| C03 | `/chat/:id` | Send, edit, delete, react, reply, thread, forward, pin, mark unread, create task, stickers, files, @/#, voice | Same | OK | — |
| C04 | `/chat/:id` | Send later, scheduled messages | Same | OK | — |
| C05 | `/chat` search | Global and per conversation | Same | OK | — |
| C06 | `/chat/:id` panes | Members, pins, files, threads | Same | OK | — |
| C07 | `/chat` | New channel, browse, join, leave, archive, edit, categories | Same | OK | — |
| C08 | `/chat/:id` menu | Notifications level, star, mark read, copy link | Same | OK | — |
| C09 | `/chat/:id` agent | Agent cards, approvals | Same (approval by "yes", as on the web) | OK | — |
| C10 | `/chat/:id` | Link previews | — | missing | Fixed: preview cards under a message (other sites' URLs, as the web picks them; tested) |
| C11 | `/chat/:id` | Attach a Drive file; large files go to Drive | — | missing | Open |
| C12 | Edit channel → Integrations | Incoming webhooks (create, URL once, edit, new URL, reading, buttons/events, delete) | Nothing | missing | Fixed: Integrations screen from the conversation menu (channel managers) |
| C13 | `/chat/:id` | Tap a person → profile card | Tap did nothing (members list opened a DM) | missing | Fixed: see G08 |
| C14 | `/chat` | Drag channels between sections; mark all read with Undo | Category in Edit channel; mark all read without Undo | missing | Open (low) |

## Drive

| Row | Route | Web has | iPhone has | Status | Fix |
|---|---|---|---|---|---|
| R01 | `/drive`, `/drive/:id` | Folders, upload, new folder, rename, move, trash, search, sort, share link, open | Same | OK | — |
| R02 | `/drive/trash` | Restore, delete forever, empty | Same | OK | — |
| R03 | `/drive` | Download several as a ZIP, upload a folder, office previews, video transcoding | — | missing | Open: iOS has no folder picker; ZIP and previews not ported |

## Settings (workspace)

| Row | Route | Web has | iPhone has | Status | Fix |
|---|---|---|---|---|---|
| S01 | `/settings` | Name, logo, leave | Same | OK | — |
| S02 | `/settings/members` | Invite (member, guest + channels), roles, transfer, remove, invitations | Same | OK | — |
| S03 | `/settings/billing` | Plan, status, checkout, portal | Status; plans on the web | OK | App Store rule |
| S04 | `/settings/emoji`, `/settings/stickers` | Add, delete | Same | OK | — |
| S05 | `/settings/file-storage` | Own bucket, usage | Same | OK | — |
| S06 | `/settings/github` | App, repositories, register | Same; registering opens the web | OK | — |
| S07 | `/settings/agentic` | Tabs You, Workspace, Agents, Routines, Skills | Tabs hidden by a condition without a message | hidden | Fixed in `iphone-agentic-tabs` (not changed here) |
| S08 | `/settings/api-tokens` | Create, revoke | Same | OK | — |
| S09 | `/settings/apps`, `/apps/:slug` | Manage apps; open in a frame | Manage; open in the browser | OK | — |
| S10 | `/settings/export`, `/settings/danger-zone` | Export ZIP; delete workspace | Same | OK | — |

## Account settings and Admin

| Row | Route | Web has | iPhone has | Status | Fix |
|---|---|---|---|---|---|
| S11 | `/profile` | Picture, name, email change, title, phone, time zone, bio, password, delete account | Same | OK | — |
| S12 | `/profile/security` | Authenticator app, recovery codes | Same | OK | — |
| S13 | `/profile/security` | Passkeys: list, add, remove | A note only | missing | Fixed: list and remove (add needs a browser's passkey prompt; the card says so) |
| S14 | `/profile/sessions`, `/appearance`, `/notifications` | Same pages | Same | OK | — |
| S15 | `/profile/agentic` | ChatGPT, MCP server, memory | Settings → Agentic → You | OK | — |
| A01 | `/admin` | Registration, Suggest a change | Same | OK | — |
| A02 | `/admin/users` | Admin, suspend, recovery link, 2FA reset, change email, reinstate | Same | OK | — |
| A03 | `/admin/email`, `/limits`, `/storage`, `/backups`, `/audit`, `/jobs` | Same pages | Same | OK | — |
| A04 | `/admin/feedback` | Feedback & fixes | Same; while off, it says how to turn it on | OK | — |

## Roles and hiding conditions

| Row | Route | Web has | iPhone has | Status | Fix |
|---|---|---|---|---|---|
| X01 | member | Settings: Members, File storage, GitHub, Agentic; no admin pages | Same (`allowedNav`) | OK | — |
| X02 | guest | Home (chat cards), Chat, account settings; other routes redirect | Same (`guestMayOpen`) | OK | — |
| X03 | conditions | Billing off, Suggest a change off, rules for members, locked page, GitHub-locked task | Each shows a message (the GitHub note is new, T09) | OK | — |

## Counts

87 rows. C13 is the chat entry point of G08 (one fix, two rows).

| Status | Rows |
|---|---|
| OK | 53 |
| n/a | 3 |
| missing | 26 |
| broken | 2 |
| hidden | 3 |
| **fixed in this branch** | **16**: G02, G07, G08, T02, T03, T04, T05, T06, T09, T11, D06, D07, C10, C12, C13, S13 |
| fixed in `iphone-agentic-tabs` | 1: S07 |

Open (not fixed here, 14): P01, T12, T15, T17, T18, T19, T20, D04, D08, D09, D10, C11, C14, R03.
