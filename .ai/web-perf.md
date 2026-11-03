# Web app CPU and memory (2026-11-02)

Why: on 2026-11-02 about 17:00 UTC a user's computer froze (black screen) while Orbit was open. The server was
healthy; a graphics driver or hardware fault is the most likely cause. This file has the measured numbers, the causes
found, what changed, and what did not change.

## How it was measured

- Scripts in `scripts/perf/` (run them again unchanged):
  - `fixtures.py`: a 150-page PDF and a 2000-row `.xlsx`.
  - `seed.mjs`: an empty Orbit gets 300 tasks, a long docs page (≈500 blocks), docs pages with the PDF, the sheet and 3
    Mermaid diagrams, a channel with 1500 messages, and an embedded app ("Products", `scripts/apps-demo/server.mjs`).
  - `serve.mjs`: serves a web build in front of a running server (same cache and security headers), so a web build can
    be measured without a new server binary.
  - `measure.mjs`: headless Chromium (Playwright + CDP). Phase A: each page in its own browser context (own renderer),
    cold cache except "warm load": load time to the first useful content, main-thread busy time (`TaskDuration`) and
    long tasks in the first 10 s. Phase B: all pages stay open; after 30 s to settle, main-thread time over 60 s, timer
    and animation-frame callbacks, JS heap after a forced GC at 1 and 5 min, running animations, backdrop-filter area,
    canvas bitmaps, and the CPU/PSS of each Chromium process type. **The idle phases ran in parallel tabs** (separate
    renderers, 2 CPUs). Phase C: 5 Orbit tabs in one context (`TABS_ONLY=1` runs only this, in a fresh browser).
  - `e2e.mjs`: browser check of the changed pages, desktop 1440x900 and phone 390x844, with screenshots.
- Setup: 2-CPU VM, `chrome-headless-shell` 1243 (software GPU), viewport 1440x900, DPR 1 unless noted. API server: the
  debug binary of main 6af5609 + app-signin-card (no API change). Before = web build of main 6af5609; after = this
  branch. Results: `/root/orbit-perf-test/results/{before,after,tabs-*}.json`.
- Limits: no real GPU driver (so no driver fault can show), and load times of pages whose code did not change move by
  up to ±60% between runs on this shared VM (the chat page: 5.2 s, then 2.3 s). Compare load times only when the
  difference is larger than that.
- "Agent chat with a long history" is a channel with 1500 messages: the agent sidecar does not run on the test instance,
  and the message list is the same component.

## Numbers

### Load (first 10 s)

| Page | Load ms before → after | Main-thread busy ms (10 s) | Long tasks ms | JS loaded |
| --- | --- | --- | --- | --- |
| First load, cold cache | 718 → 899 | 846 → 1171 | 257 → 319 | 3.7 MB (5 files) |
| First load, warm cache (= workspace home) | 592 → 387 | 711 → 435 | 104 → 81 | 0 B transferred |
| Tasks (300) | 2169 → 1712 | 1791 → 1484 | 786 → 646 | 3.7 MB |
| Docs: long page | 2257 → 2426 | 2741 → 2938 | 1971 → 1963 | 3.7 MB |
| Docs: PDF 150 pages, DPR 1 ¹ | 1418 → 1833 | 6409 → 8632 | 605 → 753 | 5.3 MB |
| Docs: PDF 150 pages, DPR 2 ¹ | 1314 → 2168 | **19047 → 15983** | **5078 → 1420** | 5.3 MB |
| Docs: spreadsheet | 1937 → 3201 | 1362 → 2169 | 613 → 1157 | 3.7 MB |
| Docs: diagrams | 2083 → 1922 | 5475 → 3505 | 3925 → 1874 | 6.1 MB (38 files) |
| Chat: 1500 messages | 5192 → 2270 | 4050 → 2225 | 2943 → 1221 | 3.7 MB |
| Apps: Products frame | 1265 → 1380 | 951 → 1063 | 265 → 460 | 3.7 MB |
| Settings | 749 → 520 | 724 → 578 | 255 → 252 | 3.7 MB |

¹ The window is the whole read-through (open the PDF, scroll to page 150 one panel height at a time, back to page 10),
about 25 s, not 10 s. Only the PDF rows have changed code; the other rows show the run-to-run noise.

### Idle (60 s after 30 s to settle; heap after GC; all pages open at once)

| Page | Main thread ms/s | Timer callbacks /s | JS heap 1 min → 5 min (MB) | Canvases with a bitmap | Canvas bitmaps |
| --- | --- | --- | --- | --- | --- |
| Workspace home | 0.4 → 0.1 | 4.0 → 0 | 8.2 → 8.2 (same after) | 0 | 0 |
| Tasks (300) | 0.5 → 0.1 | 4.0 → 0 | 20.7 → 20.7 | 0 | 0 |
| Docs: long page | 0.7 → 0.7 | 4.7 → 0.7 | 16.1 → 16.3 | 0 | 0 |
| Docs: PDF, DPR 1 | **17.1 → 8.2** | 4.7 → 0.7 | 19.0 → 19.1 | **150 → 3** | **294 MB → 6 MB** |
| Docs: PDF, DPR 2 | **102.2 → 18.3** | 4.7 → 0.7 | 18.8 → 19.0 | **150 → 3** | **1174 MB → 23 MB** |
| Docs: spreadsheet | 3.3 → 4.1 | 4.7 → 0.7 | 14.4 → 14.7 | 0 | 0 |
| Docs: diagrams | 0.9 → 0.8 | 4.7 → 0.7 | 23.4 → 23.7 | 0 | 0 |
| Chat: 1500 messages | 2.6 → 2.6 | 4.1 → 0 | 18.6 → 18.6 | 0 | 0 |
| Apps: Products frame | 0.6 → 0.1 | 4.0 → 0 | 10.0 → 10.0 | 0 | 0 |
| Settings | 0.5 → 0.1 | 4.1 → 0 | 8.2 → 8.2 | 0 | 0 |

- Heap growth over 5 min: at most 0.3 MB on every page, before and after: no leak found. No page runs
  `requestAnimationFrame` or an infinite CSS animation while idle.
- All 10 pages together: renderer PSS **2562 MB → 845 MB**; GPU process CPU over 5 min 3.1 s → 2.2 s.
- What is left on the idle PDF page is the caret blink of the focused editor (2 frames/s). Each frame commits every
  canvas layer, so the cost grew with the number of drawn pages (150 before).
- Backdrop blur: no page has a blurred area while idle. An open dialog (for example Ctrl+K) puts a `backdrop-blur-xs`
  over **100%** of the window (`components/ui/dialog.tsx`, `alert-dialog.tsx`; also `ImageViewer`).

### 5 Orbit tabs (home, tasks, long doc, chat, settings; 5 renderers; fresh browser, 2 runs each)

| | Before | After |
| --- | --- | --- |
| Main thread, all tabs, ms/s | 5.8 / 8.8 | 4.7 / 5.2 |
| Renderer PSS | 357 / 359 MB | 357 / 358 MB |
| GPU process PSS | 105 / 108 MB | 105 / 108 MB |
| JS heap per tab | 8 / 21 / 15 / 19 / 8 MB | same |

Headless Chromium counts every tab as visible; in a real browser the 4 background tabs are throttled further.

### Bundle

Main chunk `index-*.js` 2.79 MB (859 KB gzip), unchanged. Already lazy chunks: the page editor (912 KB), PDF.js
(431 KB), and Mermaid with its diagram types (elk 1.46 MB and cytoscape 435 KB among them), which load only on a page
with a diagram. The Orbit server sends assets without compression: a cold load transfers 3.7 MB.

## Causes found (largest first)

1. **PDF preview kept every page it ever drew.** A page drew when it came into view and kept its full-resolution
   canvas forever. Reading a 150-page PDF at DPR 2 left 150 canvases, 1.17 GB of bitmaps, a renderer of more than
   2 GB, and 100 ms/s of main-thread work while idle (each caret-blink frame committed 150 canvas layers). The
   observer's `800px` margin did nothing: with the window as root, the scrolling panel clips the pages first. This is
   the one place where Orbit can put a heavy and growing load on the GPU and its memory.
2. **The realtime flush timer ran 4 times a second in every tab**, also when nothing waited (≈0.3 ms/s per tab, and
   4 wake-ups per second).
3. **Main chunk size** (2.8 MB) costs ≈0.4–0.6 s of script on a cold load; a warm load uses the code cache.
4. **Dialog overlays blur the whole window** while a dialog is open.

## Changes (this branch)

| Change | Gain (measured) | Risk |
| --- | --- | --- |
| `PdfPages.tsx`: a page holds its bitmap only while it is within one panel height of the view; farther away it gives the bitmap back (size stays from CSS, so the scroll position does not move). The observer's root is the scrolling panel, so the margin works and pages draw before they come into view. | Canvas bitmaps 1174 → 23 MB (DPR 2), renderer PSS of all pages 2.6 → 0.8 GB, idle 102 → 18 ms/s, long tasks of a read-through 5.1 → 1.4 s | Low. A page scrolled back into view draws again, as on the first pass (one panel height ahead of the view). |
| `useWorkspaceEvents.ts`: the 250 ms flush timer starts with an event and stops when no refresh waits. Same retries as before (focused draft, a write, the backoff). New unit test. | Timer callbacks 4/s → 0 per idle tab; idle main thread ≈0.3 ms/s less per tab | Low; the test proves a waiting refresh still runs after the draft loses focus, and the existing backoff test still passes. |

Mobile: not ported, because mobile has neither (the PDF preview is web only; the mobile socket has no flush timer).

## Not done (ask first)

| Idea | Expected gain | Risk / why not done |
| --- | --- | --- |
| Split the main chunk by route (`lazy()` for chat, settings, admin, views, …) | ≈40–50% less JS to parse on a cold load (≈150–250 ms on this VM); nothing on a warm load | Medium: the first visit of each route shows a loading state unless chunks are prefetched; changes the router (architecture). |
| HTTP compression (gzip/brotli) of `/assets` in the Orbit server | Cold load 3.7 MB → ≈0.9 MB on the network (no CPU gain in the browser) | Low, but a server change and a `tower-http` feature; first check whether the production proxy already compresses. |
| No backdrop blur on dialog overlays (`dialog.tsx`, `alert-dialog.tsx`, `ImageViewer`) | Less GPU work while a dialog is open (a full-window blur per frame) | Visible design change. |
| Cap the PDF canvas size (e.g. DPR at most 2, or a pixel limit per page) | Small now (only 3–4 pages hold a bitmap): it helps only on DPR 3 screens or a very wide panel | Text gets softer above the cap. |
| Close the realtime socket in hidden tabs | Small: the socket is idle when nothing changes, and browsers throttle hidden tabs | Updates arrive late; reconnect cost on every tab switch. |

## Still open

- Phone: the file block's toolbar floats over the PDF panel (also on main; see
  `/root/orbit-perf-test/shots/e2e-before/phone-01-pdf-top.png`).
- The freeze itself cannot be reproduced here (no real GPU). The PDF preview was the only measured path with a GPU
  memory load that grows with use; if the user had a large PDF open, this branch removes that load.
