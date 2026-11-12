# Chat media, big chat files and file previews

Operator and developer notes for videos in chat, chat files larger than the upload limit, and previews of office
files. Code: `apps/server/src/previews/`, `apps/server/src/repositories/chat/files.rs`, `apps/server/src/repositories/drive.rs`
(`chat_folder`, `Visible`), migration `0098_chat_media_previews.sql`.

## What people see

- **File types from the content.** Orbit knows a file's type from its first bytes (`detect_media_type` in
  `crates/platform/src/files/upload.rs`): videos (MP4, QuickTime `.mov`, WebM, Matroska, AVI, 3GP), audio, HEIC, and
  office files (the ZIP or OLE container; the name only says which office type). The client's label is never used.
- **Videos in chat** show a player with a poster in the message. The server makes a playable copy (MP4, H.264 + AAC,
  `+faststart`): an H.264 video is only remuxed, any other is encoded again. Until the copy is ready the message says
  "Processing the video…". Without a copy (failed, or over the cap below) the original plays when the browser can,
  else the card offers the download. Downloads always give the original.
- **Click to open:** videos, PDFs and office files in chat open in the full-screen viewer (images keep their viewer).
- **Office previews** (xlsx, xls, csv, tsv, docx, doc, pptx, ppt, odt, ods, odp, rtf) in chat and Drive: LibreOffice
  converts the file to PDF on the server, and the viewer shows it with pdf.js, cell colors included. Without
  LibreOffice the viewer says "Preview not available" and offers the download.
- **Files larger than the chat limit** (`ORBIT_UPLOAD_MAX_FILE_BYTES`, 25 MiB by default) go to Drive →
  `Uploads/#channel`, or `Uploads/<members' names>` for a direct message. The browser sends them straight to the bucket
  in parts (presigned URLs), shows progress, and "Try again" continues an upload that dropped (only the missing parts
  are sent). No byte of such a file passes through the Orbit server. The message shows the Drive file like a chat file.
  Without Drive storage (no bucket), such files are refused with a clear message.

## Access to the chat folders in Drive

A Drive node has `chat_conversation_id`: set on the conversation's folder (`chat_root = 1`) and copied to everything
below it, also when a node moves in or out (`rescope`). A node with it is seen only by people who can read the
conversation: everyone for a public channel, the members for a private channel or a direct message. Lists, search,
trash, downloads, previews, shares and Docs references all apply this rule; a public link to a folder never shows a
chat folder nested below it. Moving a file out of a direct message's folder makes it visible where it goes. A file of
a message is also readable through chat by everyone who can read the message (as for forwards).

## The preview queue

- A `file_previews` row per stored file version (an attachment blob or a Drive blob), so a new version gets its own
  preview and none goes stale. Outputs are new attachment blobs (chat files) or objects next to the Drive file
  (`drive/<workspace>/<blob>.play.mp4`, `.poster.jpg`, `.pdf.pdf`), deleted with their source.
- Job `file.preview`: one at a time on all servers (`job_limits` can change it), low priority. ffmpeg runs with
  `nice -n 19` and 2 threads; a Drive video is read from a presigned URL (not copied to the server). LibreOffice runs
  with `nice -n 19`, in a user and network namespace (`unshare --user --map-root-user --net`: no network), with its
  own HOME, profile and temp folder per job (in the system temp folder, removed after), `timeout` and an
  address-space limit (`prlimit --as`).
- Status: `pending` → `ready`, `failed` (the converter refused the file) or `skipped` (over the cap, or the tool is
  not installed). Clients poll `GET …/preview` while it is pending. A file from before this feature gets its preview
  when it is opened in a viewer (`POST …/preview`).
- **Backfill** (job `file.preview.backfill`, queued at every start, does nothing the second time): chat files saved as
  `application/octet-stream` whose name looks like a video or office file get their real type from their bytes, and
  every chat video without a preview is queued. Nothing to run by hand; check the log line "chat file types fixed and
  video previews queued".

## Settings (`ORBIT_MEDIA_*`, all optional)

| Variable | Default | Meaning |
| --- | --- | --- |
| `ORBIT_MEDIA_TRANSCODE_MAX_BYTES` | 2147483648 (2 GiB) | Larger videos get only a poster. |
| `ORBIT_MEDIA_TRANSCODE_MAX_SECONDS` | 1200 (20 min) | Longer videos that must be encoded again get only a poster (remux of H.264 has no length cap). |
| `ORBIT_MEDIA_VIDEO_TIMEOUT_SECONDS` | 3600 | The longest one video job may run. |
| `ORBIT_MEDIA_FFMPEG_THREADS` | 2 | Threads of one ffmpeg run. |
| `ORBIT_MEDIA_DOCUMENT_MAX_BYTES` | 104857600 (100 MiB) | Larger office files get no preview. |
| `ORBIT_MEDIA_DOCUMENT_TIMEOUT_SECONDS` | 120 | The longest LibreOffice may run for one file. |
| `ORBIT_MEDIA_DOCUMENT_MEMORY_MB` | 3072 | LibreOffice's address-space limit. |
| `ORBIT_MEDIA_FFMPEG`, `ORBIT_MEDIA_FFPROBE`, `ORBIT_MEDIA_SOFFICE` | `ffmpeg`, `ffprobe`, `soffice` | Tool paths. |
| `ORBIT_MEDIA_SANDBOX` | `unshare` | `none` on a host without user namespaces (LibreOffice then has network). |

An unknown `ORBIT_MEDIA_*` key stops the server at start. The caps fit the 4-CPU / 15 GB VM: a 20-minute 1080p
re-encode at `veryfast` with 2 threads takes about real time at the lowest priority, and needs at most the output's
size in the temp folder.

## Deploy (production VM)

1. **LibreOffice** (only Calc, Writer and Impress, no GUI extras):
   `sudo apt-get install -y --no-install-recommends libreoffice-calc libreoffice-writer libreoffice-impress libreoffice-core fonts-dejavu-core fonts-liberation2`
   Check as the service user: `sudo -u orbit unshare --user --map-root-user --net true && echo ok`.
   The `orbit` unit (`NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`) allows this; do not add
   `RestrictNamespaces=` or `PrivateUsers=` to it. Rollback: `sudo apt-get purge -y 'libreoffice*' && sudo apt-get autoremove -y`
   (previews then show "Preview not available"; nothing else changes).
2. **ffmpeg** is already installed (`/usr/bin/ffmpeg`, `ffprobe`).
3. **R2 CORS** of the Drive bucket: the rule Drive already needs (`GET`, `PUT`, `HEAD` from the Orbit origin, any
   request header) covers direct uploads and the players. Also expose `Content-Range`, `Accept-Ranges`,
   `Content-Length` and `ETag` (`ExposeHeaders`), so pdf.js and players can read ranges:

   ```json
   [{"AllowedOrigins": ["https://orbit.example.com"], "AllowedMethods": ["GET", "PUT", "HEAD"],
     "AllowedHeaders": ["*"], "ExposeHeaders": ["ETag", "Content-Range", "Accept-Ranges", "Content-Length"],
     "MaxAgeSeconds": 3600}]
   ```

   Orbit does not need the `ETag` header itself (it completes multipart uploads from the bucket's part list).
4. Migration 0098 runs at start; the backfill job runs after it.
5. Without a bucket (local file storage only), chat files up to the limit and their previews work as before; files
   above the limit are refused with "need Drive storage".
