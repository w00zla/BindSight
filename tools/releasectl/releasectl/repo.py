"""Read-only git / gh queries that feed the overview.

Nothing here writes: the write commands are built and run by actions.py.
"""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass, field
from pathlib import Path

from . import core


def query(root: Path, *argv: str) -> str | None:
    """Run a read-only command; its stdout, or None when it fails."""
    try:
        p = subprocess.run(argv, cwd=root, capture_output=True, text=True,
                           timeout=60)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return p.stdout if p.returncode == 0 else None


def git(root: Path, *args: str) -> str | None:
    out = query(root, "git", *args)
    return out.strip() if out is not None else None


def current_branch(root: Path) -> str:
    """The checked-out branch, or the commit hash on a detached HEAD."""
    name = git(root, "rev-parse", "--abbrev-ref", "HEAD") or "HEAD"
    if name != "HEAD":
        return name
    return git(root, "rev-parse", "HEAD") or "HEAD"


def is_dirty(root: Path) -> bool:
    """Uncommitted changes to tracked files (untracked files do not count)."""
    return bool(git(root, "status", "--porcelain", "--untracked-files=no"))


def ref_exists(root: Path, ref: str) -> bool:
    return git(root, "rev-parse", "-q", "--verify", ref) is not None


def pending_operation(root: Path) -> str | None:
    """A merge or rebase left open (by a failed pull), or None."""
    if ref_exists(root, "MERGE_HEAD"):
        return "merge"
    for name in ("rebase-merge", "rebase-apply"):
        path = git(root, "rev-parse", "--git-path", name)
        if path and (root / path).exists():
            return "rebase"
    return None


def is_ancestor(root: Path, ref: str, of: str) -> bool:
    p = subprocess.run(["git", "merge-base", "--is-ancestor", ref, of],
                       cwd=root, capture_output=True)
    return p.returncode == 0


def ahead_behind(root: Path, ref: str, base: str) -> tuple[int, int] | None:
    out = git(root, "rev-list", "--left-right", "--count", f"{ref}...{base}")
    if not out:
        return None
    left, right = out.split()
    return int(left), int(right)


def show_file(root: Path, ref: str, path: str) -> str | None:
    out = query(root, "git", "show", f"{ref}:{path}")
    return out


def releases(root: Path) -> tuple[dict[str, str] | None, str | None]:
    """tag -> draft / pre-release / release, or (None, error)."""
    try:
        p = subprocess.run(
            ["gh", "release", "list", "--limit", "1000",
             "--json", "tagName,isDraft,isPrerelease"],
            cwd=root, capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired) as e:
        return None, str(e)
    if p.returncode != 0:
        return None, (p.stderr.strip() or "gh release list failed")
    out = {}
    for r in json.loads(p.stdout):
        state = (core.DRAFT if r["isDraft"] else
                 core.PRERELEASE if r["isPrerelease"] else core.RELEASE)
        out[r["tagName"]] = state
    return out, None


@dataclass
class Snapshot:
    branch: str = ""
    dirty: bool = False
    main_vs_origin: tuple[int, int] | None = None  # (ahead, behind)
    main_ahead_live: int | None = None  # commits on main since the latest live tag
    tags: set[str] = field(default_factory=set)
    releases: dict[str, str] | None = None
    gh_error: str | None = None
    channels_doc: dict = field(default_factory=lambda: {"channels": []})
    channels_error: str | None = None
    channels_file: bool = False  # channels.json exists on origin/main
    rows: list[core.ChannelRow] = field(default_factory=list)
    live: core.LiveInfo | None = None


def _channel_ids(root: Path, prefix: str) -> set[str]:
    out = git(root, "for-each-ref", "--format=%(refname)", prefix) or ""
    return {line[len(prefix):] for line in out.splitlines() if line}


def gather(root: Path) -> Snapshot:
    snap = Snapshot(branch=current_branch(root), dirty=is_dirty(root))
    snap.main_vs_origin = ahead_behind(root, core.MAIN, "origin/main")
    snap.tags = set((git(root, "tag", "-l", "v*") or "").split())
    snap.releases, snap.gh_error = releases(root)
    if snap.releases:
        snap.tags |= {t for t in snap.releases if core.Version.parse(t)}

    text = show_file(root, "origin/main", "channels.json")
    snap.channels_file = text is not None
    try:
        snap.channels_doc = core.load_channels(text)
    except ValueError as e:
        snap.channels_error = str(e)

    local = _channel_ids(root, "refs/heads/channel/")
    remote = _channel_ids(root, "refs/remotes/origin/channel/")
    counts = {}
    for cid in local | remote:
        ref = core.branch_of(cid) if cid in local else f"origin/{core.branch_of(cid)}"
        ab = ahead_behind(root, ref, core.MAIN)
        if ab:
            counts[cid] = ab
    snap.rows = core.build_rows(snap.channels_doc, snap.tags, snap.releases,
                                local, remote, counts)
    snap.live = core.build_live(snap.tags, snap.releases)
    if snap.live.latest and snap.live.latest.tag in snap.tags:
        n = git(root, "rev-list", "--count", f"{snap.live.latest.tag}..{core.MAIN}")
        snap.main_ahead_live = int(n) if n else None
    return snap


# --- CI build runs -----------------------------------------------------------


def run_for_tag(root: Path, tag: str) -> str | None:
    """The id of the newest build.yml run a push of `tag` started."""
    jq = f'map(select(.headBranch == "{tag}")) | .[0].databaseId // empty'
    out = query(root, "gh", "run", "list", "--workflow", "build.yml", "--event",
                "push", "--json", "databaseId,headBranch", "--jq", jq)
    return out.strip() or None if out else None


def run_view(root: Path, run_id: str) -> dict | None:
    out = query(root, "gh", "run", "view", run_id, "--json", "status,conclusion,url,jobs")
    try:
        return json.loads(out) if out else None
    except ValueError:
        return None


def run_failed_log(root: Path, run_id: str, lines: int = 40) -> str:
    """The tail of the failed steps' log, or empty."""
    out = query(root, "gh", "run", "view", run_id, "--log-failed") or ""
    return "\n".join(out.rstrip().splitlines()[-lines:])
