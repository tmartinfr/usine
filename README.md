# usine

`usine` is a software factory orchestrator: one place to see your work across the tools you use every day. It comes with an interactive terminal interface and scriptable commands; a web interface is planned.

Supported connectors so far:

| Connector | What it can do                                        |
|-----------|-------------------------------------------------------|
| Jira      | List issues (default: unresolved, assigned to me)     |
| GitLab    | List open merge requests authored by me, with their pipeline status; the terminal interface also shows those I review and have not approved yet |
| Claude    | Review a merge request in its own git worktree, from the terminal interface |

Planned: GitHub, Linear.

## Installation

From a checkout of this repository, with [Rust](https://rustup.rs) installed:

```sh
cargo install --path .
```

## Configuration

All settings live in a single TOML file, by default `~/.config/usine/config.toml`. Use `--config <PATH>` to point to another file. Environment variables are not read.

Each connector has its own section. A section is only required when you run a command of that connector.

```toml
[jira]
url = "https://<site>.atlassian.net"   # Jira Cloud site
email = "me@example.com"               # Atlassian account email
api_token = "..."                      # https://id.atlassian.com > Security > API tokens
# jql = "project = PAY AND assignee = currentUser()"   # optional, replaces the default query

[gitlab]
token = "glpat-..."                    # personal access token, read_api scope
# url = "https://gitlab.example.com"   # optional, defaults to https://gitlab.com

[git]                                  # needed to review merge requests and find their worktrees
roots = ["~/git"]                      # where your local clones live (searched 3 levels deep)
# worktrees_dir = "~/reviews"          # optional, defaults to ~/.local/share/usine/worktrees

[claude]                               # optional, defaults shown
command = "claude"
args = []                              # extra arguments, e.g. ["--model", "opus"]
review_prompt = "Review the GitLab merge request {url}. Its code is checked out in the current directory; the target branch is origin/{target_branch}."
```

The file holds credentials: restrict it with `chmod 600 ~/.config/usine/config.toml`.

## Usage

```
usine [--config <PATH>] [<COMMAND>]
```

Without a command, `usine` starts the interactive terminal interface. `--config` is accepted anywhere on the command line. Every command supports `--help`.

### Terminal interface

```
usine
usine tui
```

The screen shows these panes, one above the other:

- **Jira issues**: your issues, using the same query as `usine jira issues` (the `jql` from your config, or the default).
- **GitLab MRs**: your open merge requests, as with `usine gitlab mrs`, and the open merge requests you are a reviewer of and have not approved yet (like the review requests of the GitLab merge requests dashboard). The `ROLE` column tells them apart: `author` or `reviewer` (a merge request you authored and also review shows as `author`). The `R` column shows `*` when the merge request has a review session and `~` while one is starting. The `C` column shows `*` when it has a Claude session started with `c` (see [Claude menu](#claude-menu)). The `WORKTREE` column shows the local worktree that has the source branch of the merge request checked out (see [Worktrees](#worktrees)).
- **Review**: the live screen of the Claude review of the selected merge request, shown only while it has a review (or one is starting). When the merge request also has a Claude session started with `c`, the **Claude** pane shows its live screen next to it.
- **Notifications** (hidden by default, press `n` to show or hide it): the changes usine notified (see [Notifications](#notifications)), latest at the bottom, with their time. The last 500 are kept. Showing it moves the focus to it with the latest change selected; `Enter` opens the issue or merge request the change is about (for a finished review, its merge request when still listed).
- **API calls** (hidden by default, press `l` to show or hide it): the requests usine made to Jira and GitLab, latest at the bottom, with their time, method, HTTP status (green when successful, red otherwise, or the error when no response was received), duration and URL. The last 500 calls are kept. Credentials are never shown.

Up to 100 items are shown per pane. Data is loaded in the background when the interface starts, then refreshed automatically every minute. A pane whose connector is not configured shows which config section is missing; a pane whose source fails shows the error, while the other pane keeps working.

```
+ Jira issues (2) ------------------------------------------------------------+
|KEY      STATUS ^     PRIORITY  UPDATED     SUMMARY                           |
|OPS-7    To Do        -         2026-10-01  Rotate staging certificates       |
|PAY-123  In Progress  High      2026-10-10  Fix the payment retry loop        |
+-----------------------------------------------------------------------------+
+ GitLab MRs (2) -------------------------------------------------------------+
|R  C  ROLE ^    MR                    STATUS        PIPELINE  UPDATED     ...|
|*  *  author    group/backend/api!42  not_approved  running   2026-10-10  ...|
|      reviewer  group/web!310         mergeable     success   2026-10-09  -  |
+-----------------------------------------------------------------------------+
+ Review group/backend/api!42 [running] ++ Claude [running]  c: attach --------+
|* Reading the diff...                 ||> Fix the retry loop                 |
+--------------------------------------++-------------------------------------+
 Tab: switch  j/k: move  Enter: open  s: sort  r: refresh  a: attach  x: close  c: claude  w: view  n: notifs  l: calls  q: quit
```

| Key                     | Action                                              |
|-------------------------|-----------------------------------------------------|
| `Tab` / `Shift-Tab`     | Switch to the next / previous pane (including Notifications when shown) |
| `j` / `Down`            | Next row                                            |
| `k` / `Up`              | Previous row                                        |
| `g` / `Home`            | First row                                           |
| `G` / `End`             | Last row                                            |
| `Enter`                 | Open the selected issue or merge request in your browser (in Notifications: the one the change is about) |
| `s`                     | Choose the column to sort the pane on (see below)   |
| `r`                     | Refresh both panes now                              |
| `a`                     | Attach to the review of the selected merge request (merge request panes) |
| `x`                     | Close a finished review (merge request panes)       |
| `X`                     | Close a review even if Claude is still running (merge request panes) |
| `c`                     | Claude menu of the selected merge request: new session, resume a conversation or review (merge request panes, see below) |
| `w`                     | Switch to the next view: stacked, split, tree (see below) |
| `n`                     | Show the Notifications pane and focus it, or hide it |
| `l`                     | Show or hide the API calls pane                     |
| `q` / `Esc`             | Quit, after confirmation: `y` (or `q` again) quits, any other key cancels |
| `Ctrl-C`                | Quit immediately                                    |

#### Split view

Press `w` to show each Jira issue next to its merge requests, and `w` again to show the tree view:

```
+ Jira issues (3) -------------------------++ GitLab MRs (4) ---------------------------------------+
|KEY      STATUS ^     SUMMARY             ||R  C  ROLE      MR                    STATUS      ...|
|PAY-123  In Progress  Fix the payment ... ||*  *  author    group/backend/api!42  not_approved...|
|                                          ||      author    group/backend/api!45  mergeable   ...|
|OPS-7    To Do        Rotate staging c... ||                                                       |
|PAY-130  Testing      Drop unused fiel... ||      reviewer  group/web!310         mergeable   ...|
|                                          ||      author    group/infra!7         draft_status...|
+------------------------------------------++-------------------------------------------------------+
```

- **Jira issues** (left) and **GitLab MRs** (right) share their rows. A merge request is linked to an issue when the issue key (e.g. `PAY-123`, case ignored) appears in its source branch, its title or its description, as GitLab does for Jira. An issue with several merge requests takes one row per merge request; an issue without merge request has an empty right side. A merge request mentioning several listed issues is shown next to each of them.
- The merge requests linked to none of the listed issues come last, with an empty left side.
- Only the listed merge requests (yours and those you review) can be linked; this view makes no extra request.
- `Tab` switches between the two sides; moving on either side moves both. `Enter` opens the issue on the left, the merge request on the right. `c`, `a` and `x` act on the merge request of the selected row, which the Review pane follows.
- `s` sorts the issues (left). The merge requests follow their issue, in the order of the GitLab sort, which can be changed from the stacked view.

#### Tree view

From the split view, press `w` to show the issues and their merge requests in a single pane, each merge request on a branch below its issue, and `w` again to come back to the stacked view:

```
+ Jira issues (3)  GitLab MRs (4) -----------------------------------------------------------------+
|KEY                          STATUS ^      PRIORITY  UPDATED     SUMMARY                          |
|PAY-123                      In Progress   High      2026-10-10  Fix the payment retry loop       |
|   R C MR                    STATUS        PIPELINE  UPDATED     ROLE ^    TITLE                  |
||- * * group/backend/api!42  not_approved  running   2026-10-10  author    Add retry on payment ...|
|`-     group/backend/api!45  mergeable     success   2026-10-09  author    Retry the refunds too  |
|OPS-7                        To Do         -         2026-10-01  Rotate staging certificates      |
|PAY-130                      Testing       Medium    2026-10-08  Drop unused fields               |
|   R C MR                    STATUS        PIPELINE  UPDATED     ROLE ^    TITLE                  |
|`-     group/web!310         mergeable     success   2026-10-07  reviewer  Remove the old fields  |
|MRs of no listed issue                                                                            |
|   R C MR                    STATUS        PIPELINE  UPDATED     ROLE ^    TITLE                  |
|`-     group/infra!7         draft_status  -         2026-10-02  author    Draft: bump the base ...|
+--------------------------------------------------------------------------------------------------+
```

- The merge requests share the issue columns: the merge request under `KEY`, its merge status under `STATUS`, its pipeline under `PRIORITY`, its last update under `UPDATED`, and its role, title and worktree under `SUMMARY`. Each group of merge requests starts with the names of their columns, showing the GitLab sort.
- Merge requests are linked to issues as in the split view. A merge request mentioning several listed issues is shown below each of them.
- The merge requests linked to none of the listed issues come last, below the `MRs of no listed issue` row.
- `Enter` opens the issue or merge request of the selected row. `c`, `a` and `x` act on the selected merge request, which the Review pane follows.
- `s` sorts the issues. The merge requests follow their issue, in the order of the GitLab sort, which can be changed from the stacked view.

#### Sorting

Press `s` to sort the focused pane: the header of a column is highlighted; move it with `h` / `Left` and `l` / `Right`, then press `Enter` to sort on that column. Choosing the column the pane is already sorted on reverses the order, so `s` then `Enter` flips the current sort. `Esc` (or `s` again) cancels. The sort column shows `^` (ascending) or `v` (descending) next to its name, the selected row stays selected, and the sort is kept when the pane refreshes. By default, Jira issues are sorted by `STATUS` and merge requests by `ROLE` (yours first, then those you review); items that compare equal keep the order of the source, most recently updated first.

Ascending order means:

- `KEY`: by project, then by number (`PAY-9` before `PAY-10`).
- `STATUS` (Jira): to do, in progress, done, then by status name.
- `PRIORITY`: most important first, issues without priority last.
- `R`: running reviews, starting, finished, then merge requests without review.
- `ROLE`: authored first, then those you review.
- `MR`: by project, then by number.
- `STATUS` (GitLab): mergeable, waiting, blocked, draft, then by status name.
- `PIPELINE`: passed, in progress or waiting, failed, canceled or skipped, then merge requests without pipeline.
- `UPDATED`: oldest first.
- `SUMMARY` / `TITLE`: alphabetical, ignoring case.

#### Colors

Colors help scanning the panes:

- Jira status: blue for to do, yellow for in progress, green for done. Priority: red for high and above, yellow for medium, green or blue for low.
- GitLab status: green when the merge request can be merged, yellow while waiting (checks running, approvals missing), red when blocked (conflicts, failed pipeline, unresolved threads...). Draft merge requests are greyed out.
- Pipeline: green when passed, yellow while running or waiting (pending, manual...), red when failed, grey when canceled or skipped.
- Role: `reviewer` is highlighted in light blue, `author` is greyed out.
- Review marker: green while Claude is running, blue once finished, yellow while starting.
- Claude marker (`C` column): green while the session started with `c` runs, blue once Claude exited.
- Dates of the last 24 hours are highlighted, older ones are greyed out.
- The Review and Claude panes keep the colors of the Claude session.
- Messages at the bottom are green (done), yellow (nothing done, e.g. a review already exists) or red (error).

#### Notifications

While the terminal interface runs, usine sends a desktop notification when, at a refresh:

- a Jira issue appears in the list, changes status, or is no longer listed;
- a merge request you review appears in the list;
- the merge status of a merge request changes (`unchecked` and `checking` are ignored), or the pipeline of one of your merge requests changes;
- a merge request is no longer listed (merged, closed, or approved by you);
- a Claude review finishes.

Changes seen together are grouped in one notification; each change is also listed in the Notifications pane (`n`). Nothing is sent for the first load, nor for a pane whose previous refresh failed. Notifications use `notify-send` on Linux and `osascript` on macOS; when they are not available, nothing is sent.

The title of a pane shows `(refreshing)` while new data is being loaded, and the bottom line shows the time of the last successful update. Messages (e.g. why a review could not start) replace the key help until the next key press.

### Reviewing a merge request with Claude

Select a merge request in the GitLab pane, press `c`, then choose `3  Review with Claude` (see [Claude menu](#claude-menu)). usine then:

1. finds your local clone of the project under the `roots` of the `[git]` section;
2. fetches the merge request and its target branch, and checks the merge request out in a dedicated git worktree (`<worktrees_dir>/<project>-mr-<number>`); the files and branches of your clone are not changed;
3. starts Claude Code in that worktree with the review prompt, inside a background tmux session.

The Review pane shows what Claude is doing. Press `a` to attach to the session and interact with Claude (answer its questions, approve actions, ask for changes); detach with `Ctrl-b d` to come back to usine. If usine itself runs inside tmux, `a` switches your tmux client to the review session instead; switch back with your usual tmux keys.

Reviews keep running when you quit usine and show up again when you restart it. When Claude exits, its last screen stays visible and the review is marked `[finished]`; press `x` to close it. Closing a review does not delete its worktree, so starting a new review of the same merge request reuses and updates it.

Requirements: [tmux](https://github.com/tmux/tmux) and [Claude Code](https://claude.com/claude-code) installed, a local clone of the project (with `origin` pointing to GitLab), and SSH access to GitLab without a passphrase prompt (e.g. via ssh-agent).

### Claude menu

Select a merge request and press `c` to open its Claude menu:

```
+ group/backend/api!42 ------------------+
|1  New Claude session                   |
|2  Resume a conversation                |
|3  Review with Claude                   |
+----------------------------------------+
```

Move with `j` / `k` and press `Enter`, or press `1`, `2` or `3`. `Esc` closes the menu.

1. **New Claude session** starts Claude Code (with the `command` and `args` of the `[claude]` section) in the worktree of the merge request (see [Worktrees](#worktrees)), inside a tmux session, and attaches to it. Detach with `Ctrl-b d` to come back to usine. If usine runs inside tmux, your tmux client switches to that session instead.
2. **Resume a conversation** lists the Claude Code conversations of that worktree, most recent first, with the name given to them (`--name` or `/rename`), else the title Claude generated, else their first prompt. `Enter` resumes the selected one in a tmux session as above; `Esc` goes back to the menu.
3. **Review with Claude** starts a review (see [Reviewing a merge request with Claude](#reviewing-a-merge-request-with-claude)). It does not need a worktree.

New and resumed sessions need a worktree: without one, the first two entries show a warning. One such session runs per merge request: while it runs, `c` attaches to it directly instead of opening the menu; once Claude has exited, the menu opens again and the next session replaces it. The `C` column shows `*`, green while the session runs and blue once Claude exited, and the Claude pane, next to the Review pane, shows its live screen (its title says `c: attach` while Claude runs, `c: menu` once it exited). These sessions keep running when you quit usine.

### Jira: list issues

```
usine jira issues [--jql <JQL>] [--limit <N>] [--json]
```

| Option         | Description                                         | Default |
|----------------|-----------------------------------------------------|---------|
| `--jql <JQL>`  | JQL query to run                                    | see below |
| `--limit <N>`  | Maximum number of issues to fetch                   | 50      |
| `--json`       | Print JSON instead of a table                       | off     |

The query is chosen in this order:

1. `--jql` on the command line
2. `jql` in the `[jira]` section of the config file
3. built-in default: `assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC`

Examples:

```sh
usine jira issues
usine jira issues --jql 'project = PAY AND sprint in openSprints()' --limit 100
usine jira issues --json | jq -r '.[].key'
```

Table output:

```
KEY      TYPE  STATUS       PRIORITY  UPDATED     SUMMARY
PAY-123  Bug   In Progress  High      2026-10-10  Fix the payment retry loop
OPS-7    Task  To Do        -         2026-10-01  Rotate staging certificates
```

JSON output (one object per issue):

```json
{
  "source": "jira",
  "key": "PAY-123",
  "title": "Fix the payment retry loop",
  "status": "In Progress",
  "status_category": "in_progress",
  "priority": "High",
  "project": "PAY",
  "kind": "Bug",
  "updated": "2026-10-10T12:34:56.789+02:00",
  "url": "https://<site>.atlassian.net/browse/PAY-123"
}
```

`status_category` is `todo`, `in_progress` or `done`. `priority` is `null` when the issue has none.

### GitLab: list my open merge requests

```
usine gitlab mrs [--limit <N>] [--json]
```

| Option        | Description                                  | Default |
|---------------|----------------------------------------------|---------|
| `--limit <N>` | Maximum number of merge requests to fetch    | 50      |
| `--json`      | Print JSON instead of a table                | off     |

Lists open merge requests authored by the owner of the token, across all projects, most recently updated first.

Examples:

```sh
usine gitlab mrs
usine gitlab mrs --json | jq -r '.[] | select(.draft | not) | .url'
```

Table output:

```
MR                    STATUS        PIPELINE  TARGET   UPDATED     WORKTREE                           TITLE
group/backend/api!42  not_approved  running   main     2026-10-10  ~/git/backend/api-webhook-retry    Add retry on payment webhook
group/infra!7         draft_status  -         develop  2026-10-01  -                                  Draft: Bump dependencies
```

`STATUS` is the merge status reported by GitLab (e.g. `mergeable`, `not_approved`, `draft_status`). It often shows `unchecked` until the merge request is opened in the GitLab web UI.

`WORKTREE` is the local worktree that has the source branch of the merge request checked out, or `-` (see [Worktrees](#worktrees)).

`PIPELINE` is the status of the latest pipeline of the merge request as reported by GitLab (e.g. `success`, `running`, `pending`, `manual`, `failed`, `canceled`), or `-` when it has none. Getting it takes one extra request per merge request.

JSON output (one object per merge request):

```json
{
  "source": "gitlab",
  "reference": "group/backend/api!42",
  "project": "group/backend/api",
  "iid": 42,
  "title": "Add retry on payment webhook",
  "description": "Retries the webhook when it times out. Closes PAY-123.",
  "draft": false,
  "merge_status": "not_approved",
  "pipeline": "running",
  "source_branch": "feat/webhook-retry",
  "target_branch": "main",
  "author": "me",
  "role": "author",
  "updated": "2026-10-10T12:34:56.789+00:00",
  "url": "https://gitlab.com/group/backend/api/-/merge_requests/42",
  "worktree": "/home/me/git/backend/api-webhook-retry"
}
```

`worktree` is `null` when no worktree was found. `pipeline` is `null` when the merge request has no pipeline. `description` is empty when the merge request has none. `role` is always `author` for this command.

### Worktrees

With a `[git]` section, usine looks for the worktree of each merge request: it finds your local clone of the project under the `roots` (as for reviews), then picks the worktree of that clone, the clone itself included, that has the source branch of the merge request checked out. Worktrees in a detached state, such as the ones usine creates for reviews, are not matched. Without a `[git]` section, or when no clone or worktree is found, the worktree is empty. In the terminal interface, worktrees are looked up again at each refresh.

### Output and errors

- Tables are capped at 120 columns; the last column (summary or title) is truncated with `...`.
- An empty result prints `No issues.` or `No merge requests.`
- When something goes wrong (missing config file, missing section, invalid credentials, invalid JQL...), `usine` prints `error: ...` with the reason and exits with a non-zero code.
