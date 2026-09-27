"""Headless UI tests against a throwaway git repo (temp dir + local bare
origin; no network: gh finds no GitHub remote and the overview shows that)."""

import asyncio
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from releasectl.app import AskScreen, ConfirmScreen, PickScreen, ReleaseCtl

TOML = '[package]\nname = "bindsight"\nversion = "0.16.1"\n'
LOCK = '[[package]]\nname = "bindsight"\nversion = "0.16.1"\n'
PKG = '{\n  "name": "bindsight",\n  "version": "0.16.1",\n  "private": true\n}\n'


def git(cwd, *args):
    return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True,
                          text=True).stdout.strip()


class Repo:
    def __init__(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="releasectl-test-"))
        self.origin = self.tmp / "origin.git"
        self.work = self.tmp / "work"
        git(self.tmp, "init", "-q", "--bare", "-b", "main", str(self.origin))
        git(self.tmp, "init", "-q", "-b", "main", str(self.work))
        for k, val in [("user.name", "t"), ("user.email", "t@t"),
                       ("commit.gpgsign", "false"), ("tag.gpgsign", "false"),
                       ("pull.rebase", "false")]:
            git(self.work, "config", k, val)
        (self.work / "src-tauri").mkdir()
        self.write("src-tauri/Cargo.toml", TOML)
        self.write("src-tauri/Cargo.lock", LOCK)
        self.write("package.json", PKG)
        self.commit("init")
        git(self.work, "tag", "-a", "v0.16.1", "-m", "BindSight 0.16.1")
        git(self.work, "remote", "add", "origin", str(self.origin))
        git(self.work, "push", "-q", "origin", "main", "v0.16.1")
        git(self.work, "branch", "channel/x")
        git(self.work, "push", "-q", "origin", "channel/x")

    def write(self, path, text):
        (self.work / path).write_text(text)

    def bot_commit(self, path, text, branch="main"):
        """A commit straight onto origin, like the README / channels bot."""
        bot = self.tmp / "bot"
        if not bot.exists():
            git(self.tmp, "clone", "-q", str(self.origin), str(bot))
            for k, val in [("user.name", "bot"), ("user.email", "b@b"),
                           ("commit.gpgsign", "false")]:
                git(bot, "config", k, val)
        git(bot, "checkout", "-q", branch)
        git(bot, "pull", "-q", "origin", branch)
        (bot / path).write_text(text)
        git(bot, "add", "-A")
        git(bot, "commit", "-q", "-m", f"bot: {path}")
        git(bot, "push", "-q", "origin", branch)

    def commit(self, msg):
        git(self.work, "add", "-A")
        git(self.work, "commit", "-q", "-m", msg)

    def cleanup(self):
        shutil.rmtree(self.tmp, ignore_errors=True)


async def modal(app, pilot, kind=None):
    """Wait for the next dialog and return it."""
    for _ in range(400):
        await pilot.pause(0.02)
        if isinstance(app.screen, (ConfirmScreen, AskScreen, PickScreen)):
            if kind is None or isinstance(app.screen, kind):
                return app.screen
    raise AssertionError(f"no dialog; log: {log_text(app)}")


async def answer(app, pilot, ok, kind=ConfirmScreen, expect=None):
    screen = await modal(app, pilot, kind)
    if expect:
        assert expect in screen.title_text, (expect, screen.title_text)
    title = screen.title_text
    await pilot.click("#ok" if ok else "#cancel")
    return title


async def idle(app, pilot):
    for _ in range(400):
        await pilot.pause(0.02)
        if not app.busy:
            return
    raise AssertionError("action did not finish")


def log_text(app):
    from releasectl.app import RichLog
    return "\n".join(line.text for line in app.log_panel.lines)


def select(app, cid):
    ids = [r.id for r in app.snap.rows]
    app.query_one("DataTable").move_cursor(row=ids.index(cid))


class AppTests(unittest.TestCase):
    def setUp(self):
        self.repo = Repo()

    def tearDown(self):
        self.repo.cleanup()

    def run_app(self, body, dry_run=False):
        async def go():
            app = ReleaseCtl(self.repo.work, dry_run=dry_run)
            async with app.run_test(size=(160, 50)) as pilot:
                for _ in range(200):
                    await pilot.pause(0.02)
                    if app.snap.rows:
                        break
                await body(app, pilot)
        asyncio.run(go())

    def test_bump_channel_real(self):
        async def body(app, pilot):
            select(app, "x")
            await pilot.press("b")
            await answer(app, pilot, True, AskScreen)  # proposed version
            await answer(app, pilot, True, expect="Switch to channel/x")
            await answer(app, pilot, True, expect="Bump to 0.1.0-x.1")
            await answer(app, pilot, False, expect="Push channel/x")
            await idle(app, pilot)

        # gh finds no GitHub remote: no latest live, so the proposal is 0.1.0-x.1.
        self.run_app(body)
        w = self.repo.work
        self.assertEqual(git(w, "rev-parse", "--abbrev-ref", "HEAD"), "main")
        self.assertEqual(git(w, "log", "-1", "--format=%s", "channel/x"),
                         "Bump version to 0.1.0-x.1")
        self.assertEqual(git(w, "tag", "-l", "v0.1.0-x.1"), "v0.1.0-x.1")
        self.assertEqual(git(w, "cat-file", "-t", "v0.1.0-x.1"), "tag")
        self.assertIn('version = "0.1.0-x.1"', git(w, "show", "channel/x:src-tauri/Cargo.lock"))
        self.assertIn('"version": "0.1.0-x.1"', git(w, "show", "channel/x:package.json"))
        self.assertEqual(git(w, "status", "--porcelain"), "")
        # Nothing pushed.
        self.assertEqual(git(self.repo.origin, "tag", "-l", "v0.1.0-x.1"), "")

    def test_bump_channel_dry_run_changes_nothing(self):
        before = git(self.repo.work, "show-ref")

        async def body(app, pilot):
            select(app, "x")
            await pilot.press("b")
            await answer(app, pilot, True, AskScreen)
            await answer(app, pilot, True, expect="Switch to channel/x")
            await answer(app, pilot, True, expect="Bump to")
            await answer(app, pilot, True, expect="Push channel/x")
            await answer(app, pilot, True, expect="Watch the CI build")
            await idle(app, pilot)
            log = log_text(app)
            self.assertIn("[dry-run] $ git checkout channel/x", log)
            self.assertIn("[dry-run] $ git tag -a v0.1.0-x.1", log)
            self.assertIn("[dry-run] $ git push origin v0.1.0-x.1", log)
            self.assertIn("[dry-run] $ git checkout main", log)

        self.run_app(body, dry_run=True)
        self.assertEqual(git(self.repo.work, "show-ref"), before)
        self.assertEqual(git(self.repo.work, "status", "--porcelain"), "")

    def test_bump_live_real(self):
        w = self.repo.work
        git(w, "checkout", "-q", "channel/x")

        async def body(app, pilot):
            await pilot.press("l")
            # No release info offline: the newest live tag v0.16.1 sets the proposal.
            screen = await modal(app, pilot, AskScreen)
            self.assertEqual(screen.value, "0.17.0")
            await pilot.click("#ok")
            await answer(app, pilot, True, expect="Switch to main")
            await answer(app, pilot, True, expect="Bump to 0.17.0")
            await answer(app, pilot, False, expect="Push main")
            await idle(app, pilot)

        self.run_app(body)
        self.assertEqual(git(w, "rev-parse", "--abbrev-ref", "HEAD"), "channel/x")
        self.assertEqual(git(w, "log", "-1", "--format=%s", "main"), "Bump version to 0.17.0")
        self.assertEqual(git(w, "cat-file", "-t", "v0.17.0"), "tag")
        self.assertIn('version = "0.17.0"', git(w, "show", "main:src-tauri/Cargo.toml"))
        self.assertEqual(git(self.repo.origin, "tag", "-l", "v0.17.0"), "")

    def test_bump_live_pulls_a_diverged_main(self):
        w = self.repo.work
        self.repo.bot_commit("README.md", "v0.16.1\n")
        self.repo.write("feature.txt", "x\n")
        self.repo.commit("local work")
        git(w, "fetch", "-q", "origin")

        async def body(app, pilot):
            await pilot.press("l")
            await answer(app, pilot, True, AskScreen)
            await answer(app, pilot, True, expect="Bump to 0.17.0")
            await answer(app, pilot, False, expect="Push main")
            await idle(app, pilot)

        self.run_app(body)
        # The bot's commit and the local one are both in, then the bump.
        self.assertEqual((w / "README.md").read_text(), "v0.16.1\n")
        self.assertEqual(git(w, "log", "-1", "--format=%s"), "Bump version to 0.17.0")
        self.assertTrue(git(w, "log", "--format=%s").count("Merge branch"))
        self.assertEqual(git(w, "cat-file", "-t", "v0.17.0"), "tag")

    def test_bump_live_pull_conflict_is_undone(self):
        w = self.repo.work
        self.repo.bot_commit("README.md", "bot\n")
        self.repo.write("README.md", "local\n")
        self.repo.commit("local readme")
        head = git(w, "rev-parse", "HEAD")

        async def body(app, pilot):
            await pilot.press("l")
            await answer(app, pilot, True, AskScreen)
            await idle(app, pilot)
            log = log_text(app)
            self.assertIn("Conflicts, pull undone", log)
            self.assertIn("README.md", log)

        self.run_app(body)
        self.assertEqual(git(w, "rev-parse", "HEAD"), head)
        self.assertEqual(git(w, "status", "--porcelain"), "")
        self.assertEqual(git(w, "tag", "-l", "v0.17.0"), "")

    def test_new_channel_offers_a_pull_of_a_diverged_main(self):
        w = self.repo.work
        self.repo.bot_commit("README.md", "bot\n")
        self.repo.write("feature.txt", "x\n")
        self.repo.commit("local work")
        git(w, "checkout", "-q", "channel/x")

        async def body(app, pilot):
            await pilot.press("n")
            screen = await modal(app, pilot, AskScreen)
            screen.query_one("Input").value = "y"
            await pilot.click("#ok")
            await answer(app, pilot, True, expect="Update main from origin")
            await answer(app, pilot, True, expect="main has diverged")
            await answer(app, pilot, True, expect="Switch to main")
            await answer(app, pilot, True, expect="Create channel/y")
            await answer(app, pilot, False, expect="Push channel/y")
            await idle(app, pilot)

        self.run_app(body)
        self.assertEqual(git(w, "rev-parse", "--abbrev-ref", "HEAD"), "channel/x")
        self.assertEqual(git(w, "show", "channel/y:README.md"), "bot")
        self.assertEqual(git(w, "show", "channel/y:feature.txt"), "x")

    def test_only_remote_steps_say_they_write_to_github(self):
        from releasectl.app import remote_write
        self.assertTrue(remote_write(["git", "push", "origin", "main"]))
        self.assertTrue(remote_write(["gh", "release", "edit", "v1.0.0"]))
        self.assertFalse(remote_write(["git", "commit", "-q", "-m", "x"]))
        self.assertFalse(remote_write(["git", "tag", "-a", "v1.0.0", "-m", "x"]))

    def test_bump_live_rejects_a_channel_version_and_old_ones(self):
        from releasectl.actions import _check_live_version

        async def body(app, pilot):
            self.assertEqual(_check_live_version("0.18.0-x.1", app.snap), "x.y.z")
            self.assertEqual(_check_live_version("0.16.1", app.snap), "tag exists")
            self.assertIsNone(_check_live_version("0.17.0", app.snap))
            # The dialog shows the error and stays open.
            await pilot.press("l")
            screen = await modal(app, pilot, AskScreen)
            screen.query_one("Input").value = "0.16.1"
            await pilot.click("#ok")
            await pilot.pause(0.1)
            self.assertIn("tag exists", str(screen.query_one("#error").content))
            await pilot.click("#cancel")
            await idle(app, pilot)

        before = git(self.repo.work, "show-ref")
        self.run_app(body)
        self.assertEqual(git(self.repo.work, "show-ref"), before)

    def test_notes_without_github_stops(self):
        async def body(app, pilot):
            await pilot.press("e")
            await idle(app, pilot)
            self.assertIn("GitHub not reachable", log_text(app))

        self.run_app(body)

    def test_sync_conflict_is_aborted(self):
        w = self.repo.work
        git(w, "checkout", "-q", "channel/x")
        self.repo.write("package.json", PKG.replace("true", "false"))
        self.repo.commit("channel change")
        git(w, "checkout", "-q", "main")
        self.repo.write("package.json", PKG.replace('"private": true', '"private": 1'))
        self.repo.commit("main change")

        async def body(app, pilot):
            select(app, "x")
            await pilot.press("s")
            await answer(app, pilot, False, expect="Update main")
            await answer(app, pilot, True, expect="Switch to channel/x")
            await answer(app, pilot, True, expect="Merge main into channel/x")
            await idle(app, pilot)
            self.assertIn("Conflicts, merge aborted", log_text(app))

        self.run_app(body)
        self.assertEqual(git(w, "rev-parse", "--abbrev-ref", "HEAD"), "main")
        self.assertFalse((w / ".git" / "MERGE_HEAD").exists())
        self.assertEqual(git(w, "status", "--porcelain"), "")
        self.assertEqual(git(w, "log", "-1", "--format=%s", "channel/x"), "channel change")

    def test_remove_channel_and_branch(self):
        w = self.repo.work
        self.repo.write("channels.json", '{\n  "channels": [\n    {\n      "id": "x",\n'
                        '      "tag": "v0.17.0-x.1"\n    },\n    {\n      "id": "y",\n'
                        '      "tag": "v0.17.0-y.1"\n    }\n  ]\n}\n')
        self.repo.commit("list channels")
        git(w, "push", "-q", "origin", "main")

        async def body(app, pilot):
            select(app, "x")
            await pilot.press("x")
            await answer(app, pilot, False, expect="Update main")
            await answer(app, pilot, True, expect="Commit on main")
            await answer(app, pilot, True, expect="Push main")
            await answer(app, pilot, True, expect="Delete local channel/x")
            await answer(app, pilot, False, expect="Delete channel/x on origin")
            await idle(app, pilot)

        self.run_app(body)
        self.assertEqual(git(w, "log", "-1", "--format=%s", "origin/main"),
                         "Remove the x channel")
        self.assertEqual((w / "channels.json").read_text(),
                         '{\n  "channels": [\n    {\n      "id": "y",\n'
                         '      "tag": "v0.17.0-y.1"\n    }\n  ]\n}\n')
        self.assertEqual(git(w, "branch", "--list", "channel/x"), "")
        self.assertIn("channel/x", git(self.repo.origin, "branch", "--list", "channel/x"))

    def test_dirty_tree_refuses_switch(self):
        self.repo.write("package.json", PKG + " ")

        async def body(app, pilot):
            select(app, "x")
            await pilot.press("s")
            await idle(app, pilot)
            self.assertIn("Working tree has changes", log_text(app))

        self.run_app(body)
        self.assertEqual(git(self.repo.work, "rev-parse", "--abbrev-ref", "HEAD"), "main")


if __name__ == "__main__":
    unittest.main()
