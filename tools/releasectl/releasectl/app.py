"""The Textual UI: overview table, action buttons, a command log."""

from __future__ import annotations

import asyncio
import subprocess
from datetime import datetime, timezone
from pathlib import Path

from rich.markup import escape
from rich.text import Text
from textual import on, work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.screen import ModalScreen
from textual.widgets import (Button, DataTable, Footer, Header, Input, Label,
                             OptionList, RichLog, Static, TabbedContent, TabPane)

from . import actions, core, repo
from .actions import Cmd, FileWrite, Stop, show

# --- dialogs -----------------------------------------------------------------


def remote_write(cmd: Cmd) -> bool:
    """A step that changes origin or GitHub (a push, a gh call), not just
    the local repo."""
    return isinstance(cmd, list) and (cmd[:2] == ["git", "push"] or cmd[:1] == ["gh"])


class ConfirmScreen(ModalScreen[bool]):
    BINDINGS = [Binding("escape", "cancel", "Cancel")]

    def __init__(self, title: str, cmds: list[Cmd], note: str, sensitive: bool,
                 dry_run: bool) -> None:
        super().__init__()
        self.title_text, self.cmds, self.note = title, cmds, note
        self.sensitive, self.dry_run = sensitive, dry_run

    def compose(self) -> ComposeResult:
        dialog = Vertical(classes="dialog" + (" sensitive" if self.sensitive else ""))
        dialog.border_title = escape(self.title_text)
        if self.sensitive:
            dialog.border_subtitle = ("writes to origin / GitHub" if any(map(remote_write, self.cmds))
                                      else "commits on main, nothing leaves the repo")
        with dialog:
            text = "\n".join("$ " + show(c) for c in self.cmds)
            yield Static(escape(text), classes="cmds")
            if self.note:
                with VerticalScroll(classes="note"):
                    yield Static(escape(self.note))
            with Horizontal(classes="buttons"):
                yield Button("Print" if self.dry_run else "Run", id="ok",
                             variant="error" if self.sensitive else "primary")
                yield Button("Cancel", id="cancel")

    def on_mount(self) -> None:
        # A sensitive step starts on Cancel: Enter alone never pushes.
        self.query_one("#cancel" if self.sensitive else "#ok").focus()

    @on(Button.Pressed)
    def pressed(self, event: Button.Pressed) -> None:
        self.dismiss(event.button.id == "ok")

    def action_cancel(self) -> None:
        self.dismiss(False)


class AskScreen(ModalScreen[str | None]):
    BINDINGS = [Binding("escape", "cancel", "Cancel")]

    def __init__(self, title: str, prompt: str, value: str, validate) -> None:
        super().__init__()
        self.title_text, self.prompt, self.value = title, prompt, value
        self.validate = validate

    def compose(self) -> ComposeResult:
        dialog = Vertical(classes="dialog")
        dialog.border_title = escape(self.title_text)
        with dialog:
            yield Label(escape(self.prompt), classes="prompt")
            yield Input(self.value, id="value")
            yield Label("", id="error", classes="error")
            with Horizontal(classes="buttons"):
                yield Button("OK", id="ok", variant="primary")
                yield Button("Cancel", id="cancel")

    @on(Input.Submitted)
    @on(Button.Pressed, "#ok")
    def submit(self) -> None:
        value = self.query_one(Input).value.strip()
        error = self.validate(value) if self.validate else None
        if error:
            self.query_one("#error", Label).update(escape(error))
        else:
            self.dismiss(value)

    @on(Button.Pressed, "#cancel")
    def action_cancel(self) -> None:
        self.dismiss(None)


class PickScreen(ModalScreen[str | None]):
    BINDINGS = [Binding("escape", "cancel", "Cancel")]

    def __init__(self, title: str, options: list[str]) -> None:
        super().__init__()
        self.title_text, self.options = title, options

    def compose(self) -> ComposeResult:
        dialog = Vertical(classes="dialog")
        dialog.border_title = escape(self.title_text)
        with dialog:
            yield OptionList(*self.options)
            with Horizontal(classes="buttons"):
                yield Button("Cancel", id="cancel")

    @on(OptionList.OptionSelected)
    def chosen(self, event: OptionList.OptionSelected) -> None:
        self.dismiss(self.options[event.option_index])

    @on(Button.Pressed, "#cancel")
    def action_cancel(self) -> None:
        self.dismiss(None)


# --- the app -----------------------------------------------------------------

# Each panel carries its own button bar: (action, label, variant); the colour
# says what kind of step it is.
LIVE_BUTTONS = [("live", "Bump live", "primary")]
CHANNEL_BUTTONS = [("new", "New", "default"), ("bump", "Bump channel", "primary"),
                   ("sync", "Sync", "default"), ("finalize", "Finalize", "warning"),
                   ("remove", "Remove", "error")]
DRAFT_BUTTONS = [("notes", "Notes", "default"), ("publish", "Publish", "success")]

# The Nord palette for everything coloured by hand (states, live line, log);
# the terminal's own named colours clash with the theme.
NORD = {"red": "#BF616A", "green": "#A3BE8C", "yellow": "#EBCB8B",
        "cyan": "#88C0D0", "dim": "#7B88A1"}

# Release states in the table.
STATE_STYLE = {core.DRAFT: NORD["yellow"], core.PRERELEASE: NORD["cyan"],
               core.RELEASE: NORD["green"], core.MISSING: NORD["red"],
               core.UNKNOWN: NORD["dim"]}


def state_cell(state: str) -> Text:
    return Text(state or "-", style=STATE_STYLE.get(state, NORD["dim"]))

COLUMNS = ["Channel", "Branch", "vs main", "Listed tag", "State",
           "Newest version", "State", "Warnings"]
DRAFT_COLUMNS = ["Draft", "For"]
BUILD_COLUMNS = ["Job", "State", "Time"]

# A build job's state as an icon and a colour.
JOB_LOOK = {"queued": ("○", NORD["dim"]), "running": ("◐", NORD["yellow"]),
            "success": ("✓", NORD["green"]), "failure": ("✗", NORD["red"]),
            "cancelled": ("–", NORD["dim"]), "skipped": ("–", NORD["dim"])}


def job_state(state: str) -> Text:
    icon, colour = JOB_LOOK.get(state, ("?", NORD["dim"]))
    return Text(f"{icon} {state}", style=colour)


def button_bar(buttons: list[tuple[str, str, str]]) -> Horizontal:
    return Horizontal(*(Button(label, id=f"act-{key}", variant=variant, compact=True)
                        for key, label, variant in buttons), classes="bar")


class ReleaseCtl(App):
    TITLE = "releasectl"
    CSS = """
    Screen { background: #242933; }
    .panel { border: round $primary 35%; border-title-color: $accent;
             border-title-style: bold; padding: 0 1; background: $background;
             height: auto; margin: 0 1 1 1; }
    .panel:focus-within { border: round $primary; }
    #live-panel { margin: 1 1 1 1; }
    #channels-panel { height: 1fr; }
    /* Log and Build share the bottom as tabs. */
    #bottom { height: 30; margin: 0 1 1 1; }
    #bottom ContentSwitcher { border: round $primary 35%; background: $background;
                              height: 1fr; }
    #bottom Tab { color: $text-muted; }
    #bottom Tab.-active { color: $accent; text-style: bold; }
    #log { height: 1fr; background: $background; }
    #build-head { height: auto; padding: 0 1; }
    #build { height: 1fr; }
    #live { height: auto; }
    DataTable { background: $background; height: auto; }
    #channels { height: 1fr; }
    #drafts { max-height: 6; }
    DataTable > .datatable--header { background: #242933; color: $primary; text-style: bold; }
    DataTable > .datatable--odd-row { background: $background; }
    DataTable > .datatable--even-row { background: #333946; }
    DataTable > .datatable--cursor { background: #434C5E; color: #ECEFF4; text-style: bold; }
    DataTable:focus > .datatable--cursor { background: #4C566A; }
    /* Buttons: a raised face, the variant's colour on the text. */
    .bar { height: auto; margin-top: 1; }
    .bar Button, .buttons Button { background: #434C5E; color: #ECEFF4; border: none;
                                   min-width: 12; padding: 0 2; margin: 0 1 0 0;
                                   text-style: bold; }
    .bar Button.-primary, .buttons Button.-primary { color: $primary; }
    .bar Button.-success { color: $success; }
    .bar Button.-warning { color: $warning; }
    .bar Button.-error, .buttons Button.-error { color: $error; }
    .bar Button:hover, .buttons Button:hover { background: #4C566A; }
    .bar Button:focus, .buttons Button:focus { background: #5E81AC; color: #ECEFF4; }
    .dialog { width: 96; max-width: 100%; height: auto; max-height: 90%;
              border: round $primary; border-title-color: $primary;
              border-title-style: bold; background: $background; padding: 1 2; }
    .dialog.sensitive { border: round $error; border-title-color: $error;
                        border-subtitle-color: $error; }
    ConfirmScreen, AskScreen, PickScreen { align: center middle; background: $background 60%; }
    .prompt { color: $text-muted; }
    /* Own frame for inputs: Textual's tall border leaves half-block edges
       on the dark dialog. */
    .dialog Input { border: round $primary 50%; background: #242933;
                    width: 1fr; height: 3; margin: 0; padding: 0 1; }
    .dialog Input:focus { border: round $primary; }
    .cmds { color: $warning; background: #242933; padding: 1 2; margin-bottom: 1; }
    .note { height: auto; max-height: 16; margin-bottom: 1; color: $text-muted; }
    .error { color: $error; }
    .buttons { height: auto; margin-top: 1; }
    OptionList { height: auto; max-height: 16; border: round $primary 60%; }
    """
    BINDINGS = [
        Binding("l", "act('live')", "Bump live"),
        Binding("n", "act('new')", "New"),
        Binding("b", "act('bump')", "Bump channel"),
        Binding("s", "act('sync')", "Sync"),
        Binding("f", "act('finalize')", "Finalize"),
        Binding("x", "act('remove')", "Remove"),
        Binding("e", "act('notes')", "Notes"),
        Binding("p", "act('publish')", "Publish"),
        Binding("w", "act('watch')", "Watch build"),
        Binding("g", "act('fetch')", "Fetch"),
        Binding("r", "reload", "Refresh"),
        Binding("d", "toggle_dry", "Dry-run"),
        Binding("q", "quit", "Quit"),
    ]

    def __init__(self, root: Path, dry_run: bool = False) -> None:
        super().__init__()
        self.root = root
        self.dry_run = dry_run
        self.snap = repo.Snapshot()
        self.busy = False

    def compose(self) -> ComposeResult:
        yield Header(icon="⎇")
        with Vertical(id="live-panel", classes="panel") as panel:
            panel.border_title = "Live"
            yield Static("Loading...", id="live")
            yield button_bar(LIVE_BUTTONS)
        with Vertical(id="channels-panel", classes="panel") as panel:
            panel.border_title = "Channels"
            yield DataTable(id="channels", cursor_type="row", zebra_stripes=True)
            yield button_bar(CHANNEL_BUTTONS)
        with Vertical(id="drafts-panel", classes="panel") as panel:
            panel.border_title = "Drafts"
            yield DataTable(id="drafts", cursor_type="row", zebra_stripes=True)
            yield button_bar(DRAFT_BUTTONS)
        with TabbedContent(id="bottom", initial="log-tab"):
            with TabPane("Log", id="log-tab"):
                yield RichLog(id="log", markup=True, wrap=True)
            with TabPane("Build", id="build-tab"):
                yield Static(f"[{NORD['dim']}]No build watched. w follows the CI run of a tag.[/]",
                             id="build-head")
                yield DataTable(id="build", cursor_type="none", zebra_stripes=True)
        yield Footer()

    def on_mount(self) -> None:
        self.theme = "nord"
        # Kept as references: App.query_one searches the active screen, which
        # is a dialog while an action runs.
        self.table = self.query_one("#channels", DataTable)
        self.drafts_table = self.query_one("#drafts", DataTable)
        self.live_panel = self.query_one("#live", Static)
        self.log_panel = self.query_one(RichLog)
        self.table.add_columns(*COLUMNS)
        self.drafts_table.add_columns(*DRAFT_COLUMNS)
        self.tabs = self.query_one("#bottom", TabbedContent)
        self.build_head = self.query_one("#build-head", Static)
        self.build_table = self.query_one("#build", DataTable)
        self.build_table.add_columns(*BUILD_COLUMNS)
        self._update_subtitle()
        self.reload()

    # --- overview ---

    def _update_subtitle(self) -> None:
        self.sub_title = "DRY-RUN" if self.dry_run else ""

    @work(exclusive=True, group="reload")
    async def reload(self) -> None:
        self.snap = await asyncio.to_thread(repo.gather, self.root)
        self.render_snapshot()

    def action_reload(self) -> None:
        self.reload()

    def render_snapshot(self) -> None:
        snap = self.snap
        live = snap.live
        n = NORD
        dim = f"[{n['dim']}]"
        parts = [f"{dim}on[/] [b]{escape(snap.branch)}[/b]" +
                 (f" [{n['red']}](dirty)[/]" if snap.dirty else "")]
        if live:
            parts.append(f"{dim}live[/] [b {n['green']}]{live.latest or '-'}[/]")
            if live.newest_tag and live.newest_tag != live.latest:
                style = STATE_STYLE.get(live.newest_state, n["dim"])
                parts.append(f"{dim}newest tag[/] {live.newest_tag.tag} [{style}]{live.newest_state}[/]")
            if snap.main_ahead_live is not None:
                parts.append(f"{dim}main[/] +{snap.main_ahead_live} {dim}since {live.latest.tag}[/]")
        if snap.main_vs_origin:
            a, b = snap.main_vs_origin
            parts.append(f"{dim}vs origin[/] +{a}/-{b}")
        lines = [f"  {dim}·[/]  ".join(parts)]
        if snap.gh_error:
            lines.append(f"[{n['red']}]GitHub: {escape(snap.gh_error)}[/]")
        if snap.channels_error:
            lines.append(f"[{n['red']}]{escape(snap.channels_error)}[/]")
        elif not snap.channels_file:
            lines.append(f"{dim}No channels.json on origin/main[/]")
        self.live_panel.update("\n".join(lines))

        table = self.table
        keep = table.cursor_row
        table.clear()
        for row in snap.rows:
            branch = "/".join(x for x, has in (("local", row.local), ("origin", row.remote)) if has)
            counts = (f"+{row.ahead}/-{row.behind}" if row.ahead is not None else "")
            table.add_row(
                Text(row.id, style="bold"), branch or "-", counts,
                row.listed_tag or ("-" if not row.listed else "(none)"),
                state_cell(row.listed_state), row.newest.tag if row.newest else "-",
                state_cell(row.newest_state),
                Text("; ".join(row.warnings), style=NORD["red"]) if row.warnings else "",
                key=row.id)
        if snap.rows:
            table.move_cursor(row=min(keep, len(snap.rows) - 1))

        drafts = self.drafts_table
        keep = drafts.cursor_row
        drafts.clear()
        for tag in self.draft_tags():
            v = core.Version.parse(tag)
            kind = f"channel {v.channel}" if v and v.is_channel else "live"
            drafts.add_row(Text(tag, style="bold"), Text(kind, style=NORD["yellow"]), key=tag)
        if drafts.row_count:
            drafts.move_cursor(row=min(keep, drafts.row_count - 1))

    def selected(self) -> core.ChannelRow | None:
        table = self.table
        if not self.snap.rows or table.cursor_row < 0:
            return None
        return self.snap.rows[min(table.cursor_row, len(self.snap.rows) - 1)]

    def draft_tags(self) -> list[str]:
        return self.snap.live.drafts if self.snap.live else []

    def selected_draft(self) -> str | None:
        tags = self.draft_tags()
        row = self.drafts_table.cursor_row
        return tags[min(row, len(tags) - 1)] if tags and row >= 0 else None

    # --- actions ---

    def action_toggle_dry(self) -> None:
        self.dry_run = not self.dry_run
        self._update_subtitle()
        self.log_line(f"Dry-run {'on' if self.dry_run else 'off'}.", "yellow")

    @on(Button.Pressed)
    def button(self, event: Button.Pressed) -> None:
        if (event.button.id or "").startswith("act-"):
            self.action_act(event.button.id.removeprefix("act-"))

    def action_act(self, name: str) -> None:
        if self.busy:
            self.notify("An action is running.")
            return
        self.busy = True
        self._action_worker(name)

    @work(exclusive=True, group="action")
    async def _action_worker(self, name: str) -> None:
        try:
            await actions.ACTIONS[name](self)
        except Stop as e:
            self.log_line(str(e), "red")
        except Exception as e:  # keep the app alive, show what broke
            self.log_line(f"{type(e).__name__}: {e}", "red")
        finally:
            self.busy = False
            self.reload()

    # --- the context actions.py talks to ---

    def log_line(self, text: str, style: str = "") -> None:
        text = escape(text)
        style = NORD.get(style, style)
        self.log_panel.write(f"[{style}]{text}[/]" if style else text)

    def note(self, text: str, style: str = "") -> None:
        """actions.Ctx's log (App.log is Textual's own logger)."""
        self.log_line(text, style)

    async def confirm(self, title: str, cmds: list[Cmd], note: str = "",
                      sensitive: bool = False) -> bool:
        ok = await self.push_screen_wait(
            ConfirmScreen(title, cmds, note, sensitive, self.dry_run))
        if not ok:
            self.log_line(f"Skipped: {title}", "dim")
        return ok

    async def ask(self, title: str, prompt: str, value: str = "",
                  validate=None) -> str | None:
        return await self.push_screen_wait(AskScreen(title, prompt, value, validate))

    async def pick(self, title: str, options: list[str]) -> str | None:
        return await self.push_screen_wait(PickScreen(title, options))

    async def execute(self, cmd: Cmd) -> tuple[bool, str]:
        if self.dry_run:
            self.log_line(f"[dry-run] $ {show(cmd)}", "yellow")
            return True, ""
        self.log_line(f"$ {show(cmd)}", "cyan")
        if isinstance(cmd, FileWrite):
            try:
                (self.root / cmd.path).write_text(cmd.text, encoding="utf-8")
            except OSError as e:
                self.log_line(str(e), "red")
                return False, str(e)
            return True, ""
        proc = await asyncio.to_thread(
            subprocess.run, cmd, cwd=self.root, capture_output=True, text=True,
            stdin=subprocess.DEVNULL)
        out = (proc.stdout + proc.stderr).strip()
        if out:
            self.log_line(out, "" if proc.returncode == 0 else "red")
        return proc.returncode == 0, out

    # --- the Build tab ---

    def watch_build(self, tag: str) -> None:
        """Follow the build run of a pushed tag in the Build tab; runs in the
        background, the app stays usable."""
        self._watch_build(tag)

    def _build_marker(self, state: str | None) -> None:
        """The Build tab's title carries the run's state, seen from the Log too."""
        icon = JOB_LOOK.get(state, ("", ""))[0] if state else ""
        self.tabs.get_tab("build-tab").label = f"Build {icon}".strip()

    @work(exclusive=True, group="build")
    async def _watch_build(self, tag: str) -> None:
        self.tabs.active = "build-tab"
        self._build_marker("queued")
        self.build_table.clear()
        self.build_head.update(f"[{NORD['dim']}]waiting for the run of {escape(tag)}…[/]")
        run_id = None
        for _ in range(40):  # the run shows up a few seconds after the push
            run_id = await asyncio.to_thread(repo.run_for_tag, self.root, tag)
            if run_id:
                break
            await asyncio.sleep(3)
        if not run_id:
            self._build_marker(None)
            self.build_head.update(f"[{NORD['red']}]no build run found for {escape(tag)}[/]")
            self.log_line(f"No build run found for {tag}; check: gh run list", "red")
            return
        run = None
        while True:
            data = await asyncio.to_thread(repo.run_view, self.root, run_id)
            if data is not None:
                run = core.parse_run(data, datetime.now(timezone.utc))
                self.render_build(run, tag)
                if run.done:
                    break
            await asyncio.sleep(5)
        self.tabs.active = "log-tab"
        if run.state == "success":
            self.log_line(f"Build of {tag} succeeded; its draft is in Drafts.", "green")
            self.reload()
            return
        self.log_line(f"Build of {tag}: {run.state}. {run.url}", "red")
        if run.state == "failure":
            tail = await asyncio.to_thread(repo.run_failed_log, self.root, run_id)
            if tail:
                self.log_line(tail, "dim")

    def render_build(self, run: core.RunView, tag: str = "") -> None:
        """Jobs, and the steps of those running or failed."""
        icon, colour = JOB_LOOK.get(run.state, ("?", NORD["dim"]))
        self._build_marker(run.state)
        self.build_head.update(f"[{colour}]{icon} {run.state}[/]  [b]{escape(tag)}[/]  "
                               f"[{NORD['dim']}]{escape(run.url)}[/]")
        table = self.build_table
        table.clear()
        for job in run.jobs:
            table.add_row(Text(job.name, style="bold"), job_state(job.state),
                          core.duration(job.seconds))
            if job.expanded:
                for step in job.steps:
                    table.add_row(Text("    " + step.name), job_state(step.state),
                                  core.duration(step.seconds))

    async def suspended(self, title: str, func) -> None:
        with self.suspend():
            print(f"\n== {title} ==\n")
            try:
                func()
            except KeyboardInterrupt:
                print("\nInterrupted.")
            input("\nPress Enter to return to releasectl.")
