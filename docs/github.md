# GitHub integration

Orbit uses one GitHub App per workspace. Workspace owners and admins start App registration in workspace settings. GitHub returns the App credentials to Orbit. Orbit saves the App ID and encrypts the private key and webhook secret in its database. Each project then selects an installed repository and an issue label. No repository webhook and no Orbit API token are needed.

## Server setup

1. Give Orbit a public HTTPS URL that serves both the UI and `/api/v1/integrations/github/webhook` to GitHub. In production, set the server's canonical `ORBIT_PUBLIC_ORIGIN=https://orbit.example.com`. For local testing, use a public HTTPS tunnel to the Orbit web server with its `/api` proxy; Orbit uses the HTTPS domain open in your browser when you start App registration.
2. Generate a 32-byte key with `openssl rand -base64 32`. Store it as `ORBIT_APP_KEY`, or put it in a file and set `ORBIT_APP_KEY_FILE` to the file path. Keep this key outside the database and its backups. Orbit cannot verify webhooks or use the saved App credentials if the key is lost.
3. Restart Orbit. Do not set `GITHUB_CONNECTIONS` or the older `GITHUB_*` connection variables; Orbit no longer reads them.

For local development, `just dev` also loads a root `.env.local` file. Put `ORBIT_APP_KEY=<base64-key>` there. Keep the file private and out of Git. Open Orbit through a public HTTPS tunnel to the web server before you select **Register GitHub App**. You do not need to set `ORBIT_PUBLIC_ORIGIN` to the tunnel URL for this development flow. A plain `http://127.0.0.1` tab cannot register a GitHub App.

## Register the workspace App and connect a project

1. Open **Settings → GitHub** as a workspace owner or admin.
2. Enter a GitHub organization name if the organization should own the App, or leave it blank for a personal App. Select **Register GitHub App**. Orbit sends a private App manifest to GitHub; only the App owner can install it. Approve the registration on GitHub. GitHub sends a one-time code to Orbit, and Orbit stores the App credentials. This needs a reachable HTTPS origin.
3. Select **Install or change repositories** in workspace settings. Install the App on the GitHub account and select the repositories. GitHub sends installation events to Orbit.
4. Open **Tasks → Project settings → GitHub** and refresh after installation. Select a repository and enter a GitHub label, then save. Create the same label in GitHub. You can connect more projects to the same repository if each project uses a different label. A project has one repository connection.
5. Add that label to an issue or pull request. Orbit creates one task in the selected project. Later GitHub edits update its title and body; close and reopen events update its status. A merged pull request completes its task. A pull request closed without a merge cancels its task. Reopening it moves the task to unstarted. Removing the label pauses updates, keeps the task, and shows a **GitHub sync paused** badge. Adding the label again resumes updates on the same task. Do not put two configured project labels on one issue or pull request.

If GitHub rejects the App manifest, fix the problem and select **Register GitHub App** again. A new attempt replaces the pending registration. GitHub sends [`installation` and `installation_repositories` events](https://docs.github.com/en/webhooks/webhook-events-and-payloads#installation) to all GitHub Apps automatically; these events must not appear in the manifest's `default_events` list.

The task title and description are read-only in Orbit. Other task fields remain editable. Orbit does not send task edits to GitHub.

Comments sync both ways, for issues and for pull requests (the pull request conversation; review comments on code lines do not sync):

- A GitHub comment becomes a task comment. It shows the GitHub author's login and avatar, marked **via GitHub**, and links to the author's GitHub profile. A GitHub edit or delete changes or removes that comment in Orbit. Nobody can edit these comments in Orbit. Owners and admins can delete one in Orbit; that removes only the Orbit copy. `@Name` of a workspace member in a GitHub comment mentions that member, and followers are notified as for any comment. The notification shows the GitHub author (for example "octocat (GitHub) commented on a task you follow"), not a member, so every follower and mentioned member gets it, the workspace owner too.
- A comment written in Orbit (also by Donna or through an API token) is posted on GitHub by the workspace App, with a last line "— *name* via Orbit". An edit in Orbit edits the GitHub comment, and a delete in Orbit deletes it. Orbit sends this in the background and retries when GitHub fails, so the comment is saved in Orbit also when GitHub is unavailable. @mentions of members are sent as code (`` `@Name` ``) so they do not notify a GitHub user of that name. Files in a comment are not sent; a reply is posted as a normal comment.
- Orbit stores the GitHub comment ID of each synced comment and ignores the webhook events of the comments its App wrote, so a comment never comes back as a copy.
- While sync is paused (the sync label is removed), comments do not sync in either direction. Comments made during the pause are not copied later.

Limits of comment sync:

- Files in a comment are not sent to GitHub, and a comment with only files is not posted. A reply is posted as a normal comment.
- A comment from GitHub cannot be edited in Orbit. A delete in Orbit removes only the Orbit copy; the GitHub comment stays.
- When a comment that Orbit posted is edited or deleted on GitHub, Orbit ignores it: the Orbit comment does not change.
- Rarely, GitHub can show a comment twice: when the background job fails after GitHub accepted the comment, the retry posts it again.

Comment sync needs the App permissions **Issues: Read and write** and **Pull requests: Read and write** and the **Issue comment** event. A new App gets them from the manifest. For an App registered before, add them on GitHub (App settings → Permissions & events), then accept the new permissions on the installation.

GitHub labels sync one way to the task. On each event Orbit gives the task every GitHub label of the issue or pull request except the project's sync label. A name matches a label of the project without regard to case (`bug` uses an existing `Bug`). If the project has no such label, Orbit creates it with the GitHub color. When a label is removed on GitHub, Orbit removes it from the task, but only if the sync added it: labels added in Orbit stay, also when GitHub has the same label. Orbit never changes labels on GitHub. While sync is paused (the sync label is removed), Orbit does not change the task's labels. A label that the sync adds runs the project's label rules (Project settings → Rules), as a label added in Orbit does.

A labeled pull request creates its own Orbit task, with its GitHub title, body, and pull-request link. Orbit does not search the pull request title, body, or branch for an Orbit task URL. An unlabeled pull request does not create a task. Removing its project label pauses sync, and adding the label again resumes the same task.

If you delete a linked task in Orbit, the next GitHub event with the project label restores that same task and applies the current GitHub title, body, and status. An event without the label keeps the task deleted and pauses sync.

GitHub App registration uses the [GitHub App manifest flow](https://docs.github.com/en/apps/sharing-github-apps/registering-a-github-app-from-a-manifest). GitHub sends signed events as described in its [webhook validation guide](https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries). App installation is described in [GitHub's installation guide](https://docs.github.com/en/apps/using-github-apps/installing-your-own-github-app).
