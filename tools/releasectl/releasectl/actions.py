"""The write actions. Each one shows its commands and waits for a yes first.

Rules kept here: no force, no history rewrite, no tag or release deletion,
no branch switch on a dirty tree, back to the starting branch afterwards.
Pushes, GitHub release edits and commits on main get their own confirmation.
"""

from __future__ import annotations

import os
import shlex
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from . import core, repo
from .core import MAIN, Version, branch_of


@dataclass
class FileWrite:
    """A file written by the tool itself, shown like a command."""
    path: str
    text: str
    label: str


Cmd = list[str] | FileWrite


def show(cmd: Cmd) -> str:
    if isinstance(cmd, FileWrite):
        return f"write {cmd.path}  ({cmd.label})"
    return shlex.join(cmd)


class Ctx(Protocol):
    root: Path
    dry_run: bool
    snap: repo.Snapshot

    def note(self, text: str, style: str = "") -> None: ...
    def selected(self) -> core.ChannelRow | None: ...
    def selected_draft(self) -> str | None: ...
    async def confirm(self, title: str, cmds: list[Cmd], note: str = "",
                      sensitive: bool = False) -> bool: ...
    async def ask(self, title: str, prompt: str, value: str = "",
                  validate=None) -> str | None: ...
    async def pick(self, title: str, options: list[str]) -> str | None: ...
    async def execute(self, cmd: Cmd) -> tuple[bool, str]: ...
    async def suspended(self, title: str, func) -> None: ...
    def watch_build(self, tag: str) -> None: ...


class Stop(Exception):
    """Ends an action with a message in the log."""


async def run_all(ctx: Ctx, cmds: list[Cmd]) -> bool:
    for cmd in cmds:
        ok, _ = await ctx.execute(cmd)
        if not ok:
            return False
    return True


# --- branch handling ---------------------------------------------------------


def dirty_stop(ctx: Ctx, message: str) -> None:
    """Stop on a dirty tree; a dry run only notes it and carries on."""
    if not ctx.dry_run:
        raise Stop(message)
    ctx.note(f"[dry-run] would stop here: {message}", "yellow")


async def switch_to(ctx: Ctx, target: str, why: str) -> str | None:
    """Check out `target` (after a yes); the branch to return to, or None."""
    start = repo.current_branch(ctx.root)
    if start == target:
        return start
    if repo.is_dirty(ctx.root):
        dirty_stop(ctx, f"Working tree has changes; commit or stash before {why}.")
    back = f"afterwards: git checkout {start}"
    if not await ctx.confirm(f"Switch to {target}", [["git", "checkout", target]],
                             note=back):
        return None
    if not await run_all(ctx, [["git", "checkout", target]]):
        raise Stop(f"Could not check out {target}.")
    return start


async def switch_back(ctx: Ctx, start: str, target: str) -> None:
    if start != target:
        await ctx.execute(["git", "checkout", start])


async def pull(ctx: Ctx, branch: str) -> None:
    """Pull the checked-out `branch` from origin. A plain pull, like the old
    bump script: the bot commits to main after a publish (README,
    channels.json), so a local main often has diverged; git merges or
    rebases as configured. On a conflict the pull is undone and reported."""
    ok, _ = await ctx.execute(["git", "pull", "--no-edit", "origin", branch])
    if ok:
        return
    pending = repo.pending_operation(ctx.root)
    if pending:
        conflicts = repo.git(ctx.root, "diff", "--name-only", "--diff-filter=U") or ""
        if conflicts:
            ctx.note("Conflicts, pull undone. Files:\n  " +
                     conflicts.replace("\n", "\n  "), "red")
        await ctx.execute(["git", pending, "--abort"])
    raise Stop(f"Pulling {branch} failed; resolve by hand: git checkout {branch} && git pull origin {branch}")


async def update_main_here(ctx: Ctx) -> None:
    """Offer a pull of the checked-out main from origin."""
    cmd = ["git", "pull", "--no-edit", "origin", MAIN]
    if await ctx.confirm("Update main from origin?", [cmd],
                         note="No = continue with the local main."):
        await pull(ctx, MAIN)


async def update_main_elsewhere(ctx: Ctx) -> None:
    """Offer an update of main without checking it out: a fast-forward via
    fetch; if main has diverged, a pull on main (switch there and back)."""
    if repo.current_branch(ctx.root) == MAIN:
        return await update_main_here(ctx)
    cmd = ["git", "fetch", "origin", f"{MAIN}:{MAIN}"]
    if not await ctx.confirm("Update main from origin?", [cmd],
                             note="No = use the local main."):
        return
    if await run_all(ctx, [cmd]):
        return
    pull_cmd = ["git", "pull", "--no-edit", "origin", MAIN]
    if not await ctx.confirm("main has diverged from origin; switch to main and pull?",
                             [["git", "checkout", MAIN], pull_cmd],
                             note="No = use the local main."):
        return
    start = await switch_to(ctx, MAIN, "pulling main")
    if start is None:
        return
    try:
        await pull(ctx, MAIN)
    finally:
        await switch_back(ctx, start, MAIN)


async def merge(ctx: Ctx, ref: str, into: str, sensitive: bool) -> bool:
    """Merge `ref` into the checked-out `into`; on a conflict abort and report."""
    cmd = ["git", "merge", "--no-ff", "--no-edit", ref]
    note = "A merge commit. On a conflict the merge is aborted and reported."
    if not await ctx.confirm(f"Merge {ref} into {into}", [cmd], note=note,
                             sensitive=sensitive):
        return False
    ok, _ = await ctx.execute(cmd)
    if ok:
        return True
    conflicts = repo.git(ctx.root, "diff", "--name-only", "--diff-filter=U") or ""
    if conflicts:
        ctx.note("Conflicts, merge aborted. Files:\n  " +
                conflicts.replace("\n", "\n  "), "red")
        ctx.note(f"Resolve by hand: git checkout {into} && git merge {ref}", "red")
    await ctx.execute(["git", "merge", "--abort"])
    return False


async def offer_push(ctx: Ctx, branch: str) -> None:
    cmd = ["git", "push", "origin", branch]
    if await ctx.confirm(f"Push {branch}?", [cmd], sensitive=True):
        await ctx.execute(cmd)


# --- bump: set the version, commit, tag, push, watch -------------------------


def _read_version_files(ctx: Ctx, branch: str) -> dict[str, str]:
    texts = {}
    for path in core.VERSION_FILES:
        if ctx.dry_run:  # nothing was checked out: read the branch's copy
            text = repo.show_file(ctx.root, branch, path)
        else:
            try:
                text = (ctx.root / path).read_text(encoding="utf-8")
            except OSError:
                text = None
        if text is None:
            raise Stop(f"Cannot read {path} on {branch}.")
        texts[path] = text
    return texts


async def bump(ctx: Ctx, version: Version, branch: str) -> None:
    """The out-of-app version bump: files, commit, tag, then push and watch.

    `branch` is the checked-out branch (in a dry run the one that would be).
    """
    error = core.branch_guard(version, branch)
    if error:
        raise Stop(error)
    if not ctx.dry_run and repo.current_branch(ctx.root) != branch:
        raise Stop(f"Not on {branch}.")
    if repo.git(ctx.root, "status", "--porcelain"):
        dirty_stop(ctx, "The working tree is not clean, commit or stash first.")

    # Always tag the state origin has too: without the pull a bot commit on
    # origin would be missing from the tag and the branch push rejected.
    if repo.ref_exists(ctx.root, f"refs/remotes/origin/{branch}"):
        await pull(ctx, branch)

    if repo.ref_exists(ctx.root, f"refs/tags/{version.tag}") or version.tag in ctx.snap.tags:
        raise Stop(f"Tag {version.tag} exists already.")

    try:
        new = core.bump_texts(_read_version_files(ctx, branch), str(version))
    except ValueError as e:
        raise Stop(str(e))
    files = list(core.VERSION_FILES)
    cmds: list[Cmd] = [FileWrite(p, t, f"version {version}") for p, t in new.items()]
    cmds += [["git", "add", *files],
             ["git", "commit", "-q", "-m", core.commit_message(version)],
             ["git", "tag", "-a", version.tag, "-m", core.tag_message(version)]]
    on_main = branch == MAIN
    if not await ctx.confirm(f"Bump to {version}", cmds, sensitive=on_main,
                             note="Commits on main." if on_main else ""):
        return
    for cmd in cmds:
        ok, _ = await ctx.execute(cmd)
        if not ok:
            if not isinstance(cmd, list) or cmd[1] != "tag":
                await ctx.execute(["git", "checkout", "HEAD", "--", *files])
            else:
                ctx.note(f"Commit made, tag failed: tag it by hand "
                        f"(git tag -a {version.tag} -m '{core.tag_message(version)}').",
                        "red")
            raise Stop("Bump failed.")
    ctx.note(("[dry-run] " if ctx.dry_run else "") + f"Committed and tagged {version.tag}.", "green")

    push = [["git", "push", "origin", branch], ["git", "push", "origin", version.tag]]
    kind = f"pre-release (channel {version.channel})" if version.is_channel else "full release"
    if not await ctx.confirm(f"Push {branch} and {version.tag}?", push, sensitive=True,
                             note=f"CI builds a draft; publish it as a {kind}."):
        ctx.note("Not pushed. Later: " + " && ".join(map(show, push)))
        return
    if not await run_all(ctx, push):
        raise Stop("Push failed.")
    await offer_watch(ctx, version)


async def offer_watch(ctx: Ctx, version: Version) -> None:
    watch = [["gh", "run", "list", "--workflow", "build.yml", "--event", "push"],
             ["gh", "run", "view", "<run>", "--json", "status,conclusion,url,jobs"]]
    if not await ctx.confirm("Watch the CI build?", watch,
                             note="Read-only; the Build tab follows the run."):
        return
    if ctx.dry_run:
        for cmd in watch:
            ctx.note(f"[dry-run] $ {show(cmd)}", "yellow")
        return
    ctx.watch_build(version.tag)


async def watch(ctx: Ctx) -> None:
    """Follow the build of a pushed tag in the Build tab (read-only)."""
    tags = sorted(v for v in map(Version.parse, ctx.snap.tags) if v)
    if not tags:
        raise Stop("No version tags.")
    text = await ctx.ask("Watch the build of", "Tag", tags[-1].tag,
                         validate=lambda t: None if t in ctx.snap.tags else "no such tag")
    if text:
        ctx.watch_build(text)


# --- channels.json on main ---------------------------------------------------


def _main_channels(ctx: Ctx) -> tuple[dict, str | None]:
    """channels.json of the local main: (document, old text or None)."""
    if ctx.dry_run:
        text = repo.show_file(ctx.root, MAIN, "channels.json")
    else:
        path = ctx.root / "channels.json"
        text = path.read_text(encoding="utf-8") if path.exists() else None
    try:
        return core.load_channels(text), text
    except ValueError as e:
        raise Stop(str(e))


async def edit_channels_on_main(ctx: Ctx, edit, message: str) -> None:
    """Switch to main, apply `edit(doc)`, commit, offer the push, go back."""
    start = await switch_to(ctx, MAIN, "editing channels.json")
    if start is None:
        return
    try:
        await update_main_here(ctx)
        doc, old = _main_channels(ctx)
        try:
            new = core.dump_channels(edit(doc))
        except KeyError as e:
            raise Stop(f"Channel {e} is not listed on main.")
        if new == old:
            raise Stop("Nothing to change.")
        cmds: list[Cmd] = [FileWrite("channels.json", new, "see below"),
                           ["git", "add", "channels.json"],
                           ["git", "commit", "-q", "-m", message]]
        if not await ctx.confirm("Commit on main", cmds, note=new, sensitive=True):
            return
        for cmd in cmds:
            ok, _ = await ctx.execute(cmd)
            if not ok:
                await ctx.execute(["git", "checkout", "HEAD", "--", "channels.json"]
                              if old is not None else ["git", "rm", "-q", "--cached",
                                                       "--ignore-unmatch", "channels.json"])
                raise Stop("Commit failed.")
        await offer_push(ctx, MAIN)
    finally:
        await switch_back(ctx, start, MAIN)


# --- the actions -------------------------------------------------------------


def _need_row(ctx: Ctx) -> core.ChannelRow:
    row = ctx.selected()
    if row is None:
        raise Stop("Select a channel first.")
    return row


def _need_branch(row: core.ChannelRow) -> None:
    if not row.has_branch:
        raise Stop(f"{branch_of(row.id)} does not exist.")


async def new_channel(ctx: Ctx) -> None:
    known = {r.id for r in ctx.snap.rows}

    def check(text: str) -> str | None:
        return core.validate_channel_id(text) or ("exists" if text in known else None)

    cid = await ctx.ask("New channel", "Id", validate=check)
    if cid is None:
        return
    await update_main_elsewhere(ctx)
    branch = branch_of(cid)
    cmds: list[Cmd] = [["git", "branch", branch, MAIN]]
    if not await ctx.confirm(f"Create {branch}", cmds):
        return
    if not await run_all(ctx, cmds):
        raise Stop("Branch not created.")
    push = ["git", "push", "-u", "origin", branch]
    if await ctx.confirm(f"Push {branch}?", [push], sensitive=True):
        await ctx.execute(push)


def _check_channel_version(text: str, row: core.ChannelRow, snap: repo.Snapshot) -> str | None:
    v = Version.parse(text)
    live = snap.live.latest if snap.live else None
    if v is None or v.channel != row.id:
        return f"x.y.z-{row.id}.n"
    if v.tag in snap.tags:
        return "tag exists"
    if row.newest and v <= row.newest:
        return f"not above {row.newest}"
    if live and v.base <= live:
        return f"base not above live {live}"
    return None


async def bump_channel(ctx: Ctx) -> None:
    row = _need_row(ctx)
    _need_branch(row)
    snap = ctx.snap
    live = snap.live.latest if snap.live else None
    proposal = core.next_channel_version(row.id, core.versions_of(row.id, snap.tags), live)
    text = await ctx.ask(f"Bump the {row.id} channel", "Version", str(proposal),
                         validate=lambda t: _check_channel_version(t, row, snap))
    if text is None:
        return
    branch = branch_of(row.id)
    start = await switch_to(ctx, branch, "bumping the channel")
    if start is None:
        return
    try:
        await bump(ctx, Version.parse(text), branch)
    finally:
        await switch_back(ctx, start, branch)


def _check_live_version(text: str, snap: repo.Snapshot) -> str | None:
    v = Version.parse(text)
    live = snap.live.latest if snap.live else None
    if v is None or v.is_channel:
        return "x.y.z"
    if live and v <= live:
        return f"not above live {live}"
    return "tag exists" if v.tag in snap.tags else None


async def bump_live(ctx: Ctx) -> None:
    live = ctx.snap.live
    proposal = core.live_proposal(live.latest if live else None,
                                  live.newest_tag if live else None)
    text = await ctx.ask("Bump live", "Version", str(proposal),
                         validate=lambda t: _check_live_version(t, ctx.snap))
    if text is None:
        return
    start = await switch_to(ctx, MAIN, "bumping live")
    if start is None:
        return
    try:
        await bump(ctx, Version.parse(text), MAIN)
    finally:
        await switch_back(ctx, start, MAIN)


def _selected_draft(ctx: Ctx) -> str:
    """The draft selected in the Drafts panel."""
    snap = ctx.snap
    drafts = snap.live.drafts if snap.live else []
    if not drafts:
        raise Stop("No drafts." if snap.releases is not None else
                   f"GitHub not reachable: {snap.gh_error}")
    tag = ctx.selected_draft()
    if tag not in drafts:
        raise Stop("Select a draft first.")
    return tag


async def edit_notes(ctx: Ctx) -> None:
    """Write a draft's release notes in nano, then save them to GitHub."""
    tag = _selected_draft(ctx)
    body = repo.query(ctx.root, "gh", "release", "view", tag, "--json", "body",
                      "--jq", ".body")
    if body is None:
        raise Stop(f"Cannot read the notes of {tag}.")
    fd, name = tempfile.mkstemp(prefix=f"releasectl-{tag}-", suffix=".md")
    path = Path(name)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(body.rstrip("\n") + "\n" if body.strip() else "")
        editor = ["nano", str(path)]
        save = ["gh", "release", "edit", tag, "--notes-file", str(path)]
        if ctx.dry_run:
            for cmd in (editor, save):
                ctx.note(f"[dry-run] $ {show(cmd)}", "yellow")
            return
        await ctx.suspended(f"Release notes of {tag}",
                            lambda: subprocess.run(editor, cwd=ctx.root))
        new = path.read_text(encoding="utf-8")
        if new.strip() == body.strip():
            raise Stop("Notes unchanged.")
        if await ctx.confirm(f"Save the notes of {tag}", [save], note=new,
                             sensitive=True):
            await ctx.execute(save)
    finally:
        path.unlink(missing_ok=True)


async def publish(ctx: Ctx) -> None:
    snap = ctx.snap
    tag = _selected_draft(ctx)
    v = Version.parse(tag)
    live = snap.live.latest
    if v.is_channel:
        cmd = ["gh", "release", "edit", tag, "--draft=false", "--prerelease"]
        note = f"Pre-release; CI points the {v.channel} channel at it."
        if live and v.base <= live:
            note += f"\nWarning: base <= live {live}, the app never offers it."
    else:
        if live and v <= live:
            raise Stop(f"{tag} is not above the latest release {live.tag}.")
        cmd = ["gh", "release", "edit", tag, "--draft=false", "--prerelease=false",
               "--latest"]
        note = "Full release, marked latest."
    if await ctx.confirm(f"Publish {tag}", [cmd], note=note, sensitive=True):
        await ctx.execute(cmd)


async def sync(ctx: Ctx) -> None:
    row = _need_row(ctx)
    _need_branch(row)
    branch = branch_of(row.id)
    if repo.is_dirty(ctx.root):
        dirty_stop(ctx, "Working tree has changes; commit or stash first.")
    await update_main_elsewhere(ctx)
    start = await switch_to(ctx, branch, "syncing")
    if start is None:
        return
    try:
        merged = await merge(ctx, MAIN, branch, sensitive=False)
        if merged:
            await offer_push(ctx, branch)
    finally:
        await switch_back(ctx, start, branch)


async def finalize(ctx: Ctx) -> None:
    row = _need_row(ctx)
    _need_branch(row)
    branch = branch_of(row.id)
    ref = branch if row.local else f"origin/{branch}"
    start = await switch_to(ctx, MAIN, "finalizing")
    if start is None:
        return
    try:
        await update_main_here(ctx)
        if not await merge(ctx, ref, MAIN, sensitive=True):
            return
        live = ctx.snap.live.latest if ctx.snap.live else None
        proposal = core.next_live(live, row.newest)
        text = await ctx.ask("Bump live?", "Version (Esc = not now)",
                             str(proposal),
                             validate=lambda t: _check_live_version(t, ctx.snap))
        if text is None:
            await offer_push(ctx, MAIN)
            return
        await bump(ctx, Version.parse(text), MAIN)
        ctx.note(f"Next: remove the {row.id} channel once the release is published.")
    finally:
        await switch_back(ctx, start, MAIN)


async def remove_channel(ctx: Ctx) -> None:
    row = _need_row(ctx)
    if not row.listed:
        raise Stop(f"{row.id} is not in channels.json.")
    await edit_channels_on_main(ctx, lambda doc: core.remove_channel(doc, row.id),
                                f"Remove the {row.id} channel")
    await offer_branch_delete(ctx, row.id)


async def offer_branch_delete(ctx: Ctx, cid: str) -> None:
    """Delete channel/<id> locally / on origin, each only when merged into main."""
    branch = branch_of(cid)
    root = ctx.root
    if repo.ref_exists(root, f"refs/heads/{branch}"):
        if repo.current_branch(root) == branch:
            ctx.note(f"{branch} is checked out; not deleting it.")
        elif not repo.is_ancestor(root, branch, MAIN):
            ctx.note(f"{branch} is not merged into main; kept.")
        else:
            cmd = ["git", "branch", "-d", branch]
            if await ctx.confirm(f"Delete local {branch}?", [cmd]):
                await ctx.execute(cmd)
    if repo.ref_exists(root, f"refs/remotes/origin/{branch}"):
        if not repo.is_ancestor(root, f"origin/{branch}", "origin/main"):
            ctx.note(f"origin/{branch} is not merged into origin/main; kept.")
        else:
            cmd = ["git", "push", "origin", "--delete", branch]
            if await ctx.confirm(f"Delete {branch} on origin?", [cmd], sensitive=True):
                await ctx.execute(cmd)


async def fetch(ctx: Ctx) -> None:
    # No --prune together with --tags: that would delete local tags.
    cmds: list[Cmd] = [["git", "fetch", "--prune", "origin"],
                       ["git", "fetch", "--tags", "origin"]]
    if await ctx.confirm("Fetch origin", cmds, note="Remote refs and tags only."):
        await run_all(ctx, cmds)


ACTIONS = {
    "new": new_channel,
    "bump": bump_channel,
    "live": bump_live,
    "notes": edit_notes,
    "publish": publish,
    "sync": sync,
    "finalize": finalize,
    "remove": remove_channel,
    "fetch": fetch,
    "watch": watch,
}
