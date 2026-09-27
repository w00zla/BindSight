# releasectl

The TUI for BindSight's releases and update channels: version bumps, tags,
publishing, release notes and `channels.json`. The release model behind it
(live line on `main`, channels on `channel/<id>`, CI) is in `CLAUDE.md`,
section "Releases and updates".

## Start

```sh
tools/releasectl.sh            # creates tools/releasectl/.venv on first run
tools/releasectl.sh --dry-run  # prints every command instead of running it
```

Needs Python 3, `git` and a logged-in `gh`. `d` toggles dry-run at any time.

## Screen

Three panels, each one's buttons acting on what it shows, and two tabs at
the bottom:

- **Live**: the checked-out branch, the latest live release, main vs
  origin — button Bump live.
- **Channels**: one row per channel — branch (local / origin, ahead /
  behind main), the tag `channels.json` lists, the newest channel version,
  their release states, warnings — buttons New, Bump channel, Sync,
  Finalize, Remove for the selected row.
- **Drafts**: the unpublished releases, live and channel — buttons Notes
  and Publish for the selected draft.
- **Log** tab: every command run and its output.
- **Build** tab: the CI run of a pushed tag, one row per job with its
  state and time, the steps of the jobs running or failed below them,
  updated every few seconds in the background. Watching switches to it,
  the end back to the Log; the tab title carries the run's state. A
  success reloads the drafts, a failure puts the failed steps' log tail
  into the Log.

Every action shows the exact commands and waits for a yes. Pushes, GitHub
release edits and commits on main ask separately and start on Cancel. The
tool never force-pushes, rewrites history or deletes tags / releases, never
switches branches on a dirty tree, and returns to the branch it started on.

## Actions

| Key | Action | What it does |
|---|---|---|
| `n` | New | Creates `channel/<id>` from main, offers the push. |
| `b` | Bump channel | Next version of the selected channel (`x.y.z-<id>.<n>`) on its branch: pull, version in Cargo.toml / Cargo.lock / package.json, commit, tag, push, optional build watch. |
| `l` | Bump live | The same on main for a live version (`x.y.z`). |
| `e` | Notes | Opens the selected draft's release notes in `$VISUAL` / `$EDITOR`, then saves them to GitHub. |
| `p` | Publish | Publishes the selected draft: a channel version as pre-release (CI then moves the channel in `channels.json`), a live version as the latest full release. |
| `s` | Sync | Merges main into the selected channel branch (live fixes, or a new base after a live release). A conflict aborts the merge and lists the files. |
| `f` | Finalize | Merges the selected channel into main, then offers Bump live. |
| `x` | Remove | Removes the channel from `channels.json` on main; offers to delete its merged branch. |
| `w` | Watch build | Follows the CI run of a pushed tag in the Build tab (offered after every push, too). |
| `g` | Fetch | `git fetch` of branches and tags. |
| `r` | Refresh | Re-reads git and GitHub. |
| `q` | Quit | |

## Typical runs

**Live release**: `l` → push → CI draft → `e` notes → `p` publish.

**Channel**:

1. `n` → `channel/<id>`, work there.
2. `b` → push → CI draft → `e` → `p`; testers pick the channel in the app's
   update dialog.
3. More fixes: `b` again. Live fixes they need: `s`.
4. Test validated: `f` (merge + Bump live) → `e` → `p`.
5. `x` removes the channel; its testers fall back to stable and get the
   final.

## Tests

```sh
tools/releasectl/.venv/bin/python -m unittest discover -s tools/releasectl/tests -t tools/releasectl
```

The pure logic (`core.py`) has unit tests; the UI tests run against a
throwaway repo with a local origin, never against GitHub.
