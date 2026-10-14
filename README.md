# usine

`usine` is a software factory orchestrator: one place to see your work across the tools you use every day. It comes with an interactive terminal interface and scriptable commands; a web interface is planned.

Supported connectors so far:

| Connector | What it can do                                        |
|-----------|-------------------------------------------------------|
| Jira      | List issues (default: unresolved, assigned to me)     |
| GitLab    | List open merge requests authored by me               |

Planned: GitHub, Linear, Claude instances, git worktrees.

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

The screen shows two panes, one above the other:

- **Jira issues**: your issues, using the same query as `usine jira issues` (the `jql` from your config, or the default).
- **GitLab MRs**: your open merge requests, as with `usine gitlab mrs`.

Up to 100 items are shown per pane. Data is loaded in the background when the interface starts, then refreshed automatically every 5 minutes. A pane whose connector is not configured shows which config section is missing; a pane whose source fails shows the error, while the other pane keeps working.

```
+ Jira issues (2) ------------------------------------------------------------+
|KEY      STATUS       UPDATED     SUMMARY                                     |
|PAY-123  In Progress  2026-10-10  Fix the payment retry loop                  |
|OPS-7    To Do        2026-10-01  Rotate staging certificates                 |
+-----------------------------------------------------------------------------+
+ GitLab MRs (1) -------------------------------------------------------------+
|MR                    STATUS        UPDATED     TITLE                         |
|group/backend/api!42  not_approved  2026-10-10  Add retry on payment webhook  |
+-----------------------------------------------------------------------------+
 Tab: switch  j/k: move  g/G: first/last  Enter: open  r: refresh  q: quit  updated 11:42
```

| Key                     | Action                                              |
|-------------------------|-----------------------------------------------------|
| `Tab` / `Shift-Tab`     | Switch between the two panes                        |
| `j` / `Down`            | Next row                                            |
| `k` / `Up`              | Previous row                                        |
| `g` / `Home`            | First row                                           |
| `G` / `End`             | Last row                                            |
| `Enter`                 | Open the selected issue or merge request in your browser |
| `r`                     | Refresh both panes now                              |
| `q` / `Esc` / `Ctrl-C`  | Quit                                                |

The title of a pane shows `(refreshing)` while new data is being loaded, and the bottom line shows the time of the last successful update.

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
  "priority": "High",
  "project": "PAY",
  "kind": "Bug",
  "updated": "2026-10-10T12:34:56.789+02:00",
  "url": "https://<site>.atlassian.net/browse/PAY-123"
}
```

`priority` is `null` when the issue has none.

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
MR                     STATUS        TARGET   UPDATED     TITLE
group/backend/api!42   not_approved  main     2026-10-10  Add retry on payment webhook
group/infra!7          draft_status  develop  2026-10-01  Draft: Bump dependencies
```

`STATUS` is the merge status reported by GitLab (e.g. `mergeable`, `not_approved`, `draft_status`). It often shows `unchecked` until the merge request is opened in the GitLab web UI.

JSON output (one object per merge request):

```json
{
  "source": "gitlab",
  "reference": "group/backend/api!42",
  "project": "group/backend/api",
  "title": "Add retry on payment webhook",
  "draft": false,
  "merge_status": "not_approved",
  "source_branch": "feat/webhook-retry",
  "target_branch": "main",
  "author": "me",
  "updated": "2026-10-10T12:34:56.789+00:00",
  "url": "https://gitlab.com/group/backend/api/-/merge_requests/42"
}
```

### Output and errors

- Tables are capped at 120 columns; the last column (summary or title) is truncated with `...`.
- An empty result prints `No issues.` or `No merge requests.`
- When something goes wrong (missing config file, missing section, invalid credentials, invalid JQL...), `usine` prints `error: ...` with the reason and exits with a non-zero code.
