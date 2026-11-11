# What's new in Orbit

<!--
Donna reads this file for "what's new?" (apps/agent/src/version.ts). Every deploy adds one entry on top, before the
build: "## YYYY-MM-DD" (the deploy date), then 1-4 short "- " lines about what people can see or do. Write for team
members, not developers. Keep the 8 newest entries; the prompt shows the 5 newest.
-->

## 2026-11-11
- Donna has her own card in Settings → Agentic → Agents, first and marked Default: owners and admins change her instructions and model for the workspace there, and installation admins her name and avatar (Admin → AI agent now has only the default model). Each agent card, Donna's too, ticks the connected services (for example TikTok or Shopify) that the agent may use; a new service is on for Donna and off for the other agents until ticked.
- Insert from Drive in Docs: type "/" → Insert from Drive and pick a file. The page always shows the latest version from Drive (images and videos on the page, other files as a card that opens the preview). People without access to the file see a locked card. Ask Donna to add, change or remove a Drive file on a page, or which pages show a file.
- More agents: owners and admins create agents next to Donna in Settings → Agentic → Agents, each with a name, avatar, instructions, tools, model and the channels where it works. @mention one in those channels or send it a direct message; it works with your permissions and asks for your yes before changes, like Donna.
- Answer level: pick Short, Normal or Thorough in Account settings → Agentic. It sets how much Donna and every other agent thinks and writes for you.
- Routines can use any agent: pick it when you edit the routine in Settings → Agentic → Routines.
- Routines: ask Donna for a recurring job ("every weekday at 9, summarize yesterday in #support"). She does it as you, with your permissions, and posts the result where you chose. See, pause, edit and delete them in Settings → Agentic → Routines.
- Reminders: "remind me at 5pm to call Ana" and Donna sends you a direct message then.
- Send later: the arrow next to Send in chat → Send later…. Your message is posted as you at that time; edit or cancel it in Scheduled messages. Donna can schedule one for you too.
- Follow tasks: you follow a task you created, are assigned to, comment on or are @mentioned in. Each new comment and status change shows in Activity and as a desktop or phone notification (not your own). The Followers field of a task lists who follows and has Follow / Unfollow. Turn it off in Account settings → Notifications → "Comments and status changes on tasks I follow".
- Ask Donna from any page: a small "Ask Donna…" box at the bottom of Tasks, Docs, Drive, Activity, settings and the other pages (not in Chat; web and desktop app; it shows once your ChatGPT is connected in Account settings → Agentic). Press Enter and Donna answers in a panel on the right while you stay on the page. Close the panel and open it again: the conversation is still there.
- Donna knows the page you asked from and the task, Docs page or Drive folder open on it, so "this" means that item. The conversation is your direct message with Donna, so "Open in Chat" shows it in Chat. Changes still ask for your yes, as in Chat.

## 2026-11-10
- Guests: owners and admins can invite outside people to chosen private channels (Settings → Members → Invite, role Guest). A guest sees only those channels, direct messages with people there and their account settings: no public channels, tasks, Docs, Drive or Donna.
- Owners and admins can connect Donna to remote MCP servers that use OAuth, for example TikTok Social: Settings → Agentic → Workspace → Add tool, credential "Sign in with OAuth (Connect)", then sign in and click Authorize. The row shows who is connected and when to reconnect.
- Phones: Settings, Account settings and Admin open as a menu of their pages. The bottom bar has Drive instead of Mail. Task boards stay fast with many tasks, and every card is there when you scroll. A change to many tasks at once shows in each task's activity. A closed GitHub issue or merged pull request moves its task to Done. Comments lose empty lines at the start and end, and sending a comment no longer blanks the page. Loading shows a small ring spinner. In Drive, a click opens a file or folder; tick the box (or Ctrl/Shift+click) to select.
- Chat on phones: Donna's cards and wide tables fit the screen and scroll sideways, the day chip no longer covers text, and repeated steps show once with a count. Desktop app 0.1.3 for Windows: the taskbar shows unread messages, and "Sign in to app" opens its window.

## 2026-11-09
- Donna can look at images, videos and scanned PDFs in Drive, edit them (for example trim a video, or crop, resize or convert an image) and save the result as a new Drive file. She cannot hear sound.
- Donna knows the current time and date of the person who writes to her, so "tomorrow" and "next week" are right, and she knows which Orbit version runs and what is new.
- Donna can answer with cards: tables, charts, task lists, short forms and buttons. They build live while she writes, also in the phone app.
- The Orbit logo is now a pink pixel "W", and the Orbit desktop app for Windows is ready: Download desktop app in the account menu, or https://orbit.wellrox.com/downloads/desktop/. The app icon picker (Settings → Apps) has new ecommerce icons; on a phone, the top bar stays visible on the chat list.

## 2026-11-07
- Donna can work in Drive: find, read, move and rename files, create folders and save text files. She moves items to the trash or makes a public link only after you say yes.
- The wellrox-cloud files are now in Drive.

## 2026-11-06
- The Drive folder tree shows in the app sidebar.
- API tokens can cover all projects, also future ones, and give access to the public REST API and MCP.
- Chat incoming webhooks: other tools can post to a channel, also with Slack Block Kit and buttons.
- Agentic moved to workspace settings.
