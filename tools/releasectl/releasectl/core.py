"""Pure release logic: versions, channels.json, version bumps, the overview.

No Textual, no git, no network here: everything takes and returns plain
data, so it is covered by unit tests.
"""

from __future__ import annotations

import copy
import json
import re
from dataclasses import dataclass, field
from functools import total_ordering

# Same shapes as the app and CI: x.y.z (live) or x.y.z-<id>.<n> (channel).
_NUM = r"(0|[1-9][0-9]*)"
_ID = r"[a-z][a-z0-9-]{0,31}"
VERSION_RE = re.compile(rf"^{_NUM}\.{_NUM}\.{_NUM}(?:-({_ID})\.{_NUM})?$")
CHANNEL_ID_RE = re.compile(rf"^{_ID}$")

MAIN = "main"
BRANCH_PREFIX = "channel/"


# --- versions ---------------------------------------------------------------


@total_ordering
@dataclass(frozen=True)
class Version:
    major: int
    minor: int
    patch: int
    channel: str | None = None  # None = live version
    n: int | None = None

    @classmethod
    def parse(cls, text: str) -> Version | None:
        """Parse `x.y.z` or `x.y.z-<id>.<n>`, with or without a leading `v`."""
        if text.startswith("v"):
            text = text[1:]
        m = VERSION_RE.match(text)
        if not m:
            return None
        major, minor, patch, channel, n = m.groups()
        if channel == "stable":
            return None
        return cls(int(major), int(minor), int(patch), channel,
                   int(n) if n is not None else None)

    @property
    def base(self) -> Version:
        return Version(self.major, self.minor, self.patch)

    @property
    def is_channel(self) -> bool:
        return self.channel is not None

    @property
    def tag(self) -> str:
        return f"v{self}"

    def __str__(self) -> str:
        core = f"{self.major}.{self.minor}.{self.patch}"
        return f"{core}-{self.channel}.{self.n}" if self.is_channel else core

    def _key(self) -> tuple:
        # Semver: a pre-release sorts before its release; pre-release
        # identifiers compare one by one, numeric < alphanumeric, numbers
        # numerically, strings in ASCII order.
        core = (self.major, self.minor, self.patch)
        if not self.is_channel:
            return core + (1, ())
        return core + (0, ((1, self.channel), (0, self.n)))

    def __lt__(self, other: Version) -> bool:
        return self._key() < other._key()


def validate_channel_id(text: str) -> str | None:
    """Return an error message, or None when `text` is a usable channel id."""
    if not CHANNEL_ID_RE.match(text):
        return "a-z first, then a-z 0-9 -, at most 32"
    if text == "stable":
        return "'stable' is built in"
    return None


def branch_of(channel_id: str) -> str:
    return BRANCH_PREFIX + channel_id


def next_minor(v: Version) -> Version:
    return Version(v.major, v.minor + 1, 0)


def live_proposal(latest: Version | None, newest_tag: Version | None) -> Version:
    """Propose a plain live bump: the next minor after the latest live
    release or the newest live tag, whichever is higher."""
    base = max((v for v in (latest, newest_tag) if v), default=None)
    return next_minor(base) if base else Version(0, 1, 0)


def next_channel_version(channel_id: str, existing: list[Version],
              latest_live: Version | None) -> Version:
    """Propose the next version of a channel.

    The next n on the current base while that base is above the latest live
    version; otherwise (none yet, or the base was released) the next minor
    after the latest live version, n = 1.
    """
    own = sorted(v for v in existing if v.channel == channel_id)
    newest = own[-1] if own else None
    if newest and (latest_live is None or newest.base > latest_live):
        return Version(newest.major, newest.minor, newest.patch, channel_id,
                       newest.n + 1)
    start = next_minor(latest_live) if latest_live else Version(0, 1, 0)
    return Version(start.major, start.minor, start.patch, channel_id, 1)


def next_live(latest_live: Version | None, channel_version: Version | None) -> Version:
    """Propose the live version that finalizes a channel version: its base if that is
    above the latest live version, else the next minor."""
    if channel_version and (latest_live is None or channel_version.base > latest_live):
        return channel_version.base
    return next_minor(latest_live) if latest_live else Version(0, 1, 0)


# --- bump (the version in the app's files) -----------------------------------


def branch_guard(version: Version, branch: str) -> str | None:
    """A live version is tagged only on main, a channel version only on channel/<id>."""
    if not version.is_channel and branch != MAIN:
        return f"a live version is tagged on main, this is '{branch}'"
    if version.is_channel and branch != branch_of(version.channel):
        return (f"a {version.channel} version is tagged on "
                f"{branch_of(version.channel)}, this is '{branch}'")
    return None


_CARGO_VERSION_RE = re.compile(r'^version = "(.*)"$', re.M)


def cargo_toml_version(text: str) -> str | None:
    """The package version: the first `version = ` line of Cargo.toml."""
    m = _CARGO_VERSION_RE.search(text)
    return m.group(1) if m else None


def set_cargo_toml(text: str, current: str, new: str) -> str:
    line = f'version = "{current}"'
    pattern = re.compile(rf"^{re.escape(line)}$", re.M)
    return pattern.sub(f'version = "{new}"', text, count=1)


def set_cargo_lock(text: str, current: str, new: str) -> str:
    """The version line right after our own package's name."""
    pattern = re.compile(
        rf'^(name = "bindsight"\n)version = "{re.escape(current)}"$', re.M)
    return pattern.sub(rf'\g<1>version = "{new}"', text, count=1)


def set_package_json(text: str, current: str, new: str) -> str:
    """The top-level version (two-space indent)."""
    pattern = re.compile(rf'^  "version": "{re.escape(current)}",$', re.M)
    return pattern.sub(f'  "version": "{new}",', text, count=1)


VERSION_FILES = {
    "src-tauri/Cargo.toml": set_cargo_toml,
    "src-tauri/Cargo.lock": set_cargo_lock,
    "package.json": set_package_json,
}


def bump_texts(texts: dict[str, str], new: str) -> dict[str, str]:
    """Set `new` in every version file; raise ValueError if one stays unchanged.

    `texts` maps the paths of VERSION_FILES to their contents.
    """
    current = cargo_toml_version(texts["src-tauri/Cargo.toml"])
    if current is None:
        raise ValueError("no version line in src-tauri/Cargo.toml")
    if current == new:
        raise ValueError(f"the version is {new} already")
    out = {}
    for path, setter in VERSION_FILES.items():
        changed = setter(texts[path], current, new)
        if changed == texts[path] or f'"{new}"' not in changed:
            raise ValueError(f"{path} was not updated")
        out[path] = changed
    return out


def commit_message(version: Version) -> str:
    return f"Bump version to {version}"


def tag_message(version: Version) -> str:
    return f"BindSight {version}"


# --- channels.json -----------------------------------------------------------

_KEY_ORDER = ("id", "tag")


def load_channels(text: str | None) -> dict:
    """Parse channels.json; a missing file (None) is an empty channel list."""
    if text is None:
        return {"channels": []}
    doc = json.loads(text)
    if not isinstance(doc, dict) or not isinstance(doc.get("channels", []), list):
        raise ValueError("channels.json: expected {\"channels\": [...]}")
    doc.setdefault("channels", [])
    for entry in doc["channels"]:
        if not isinstance(entry, dict) or not isinstance(entry.get("id"), str):
            raise ValueError("channels.json: every channel needs an id")
    return doc


def dump_channels(doc: dict) -> str:
    """Serialize with 2-space indent, trailing newline, keys id/tag first."""
    out = {k: v for k, v in doc.items() if k != "channels"}
    entries = []
    for entry in doc.get("channels", []):
        ordered = {k: entry[k] for k in _KEY_ORDER if k in entry}
        ordered.update({k: v for k, v in entry.items() if k not in _KEY_ORDER})
        entries.append(ordered)
    out = {"channels": entries, **out}
    return json.dumps(out, indent=2, ensure_ascii=False) + "\n"


def find_channel(doc: dict, channel_id: str) -> dict | None:
    return next((e for e in doc["channels"] if e["id"] == channel_id), None)


def remove_channel(doc: dict, channel_id: str) -> dict:
    if find_channel(doc, channel_id) is None:
        raise KeyError(channel_id)
    new = copy.deepcopy(doc)
    new["channels"] = [e for e in new["channels"] if e["id"] != channel_id]
    return new


# --- overview ----------------------------------------------------------------

DRAFT, PRERELEASE, RELEASE, MISSING, UNKNOWN = (
    "draft", "pre-release", "release", "missing", "?")


def release_state(tag: str | None, releases: dict[str, str] | None) -> str:
    """`releases` maps tag -> state; None means GitHub could not be asked."""
    if tag is None:
        return ""
    if releases is None:
        return UNKNOWN
    return releases.get(tag, MISSING)


def latest_live(tags: set[str], releases: dict[str, str] | None) -> Version | None:
    """The highest live version published as a full release."""
    if releases is None:
        return None
    lives = [v for v in map(Version.parse, tags)
             if v and not v.is_channel and releases.get(v.tag) == RELEASE]
    return max(lives, default=None)


def versions_of(channel_id: str, tags: set[str]) -> list[Version]:
    return sorted(v for v in map(Version.parse, tags)
                  if v and v.channel == channel_id)


@dataclass
class ChannelRow:
    id: str
    listed: bool = False
    listed_tag: str | None = None
    listed_state: str = ""
    local: bool = False
    remote: bool = False
    ahead: int | None = None  # commits on the branch that main lacks
    behind: int | None = None  # commits on main the branch lacks
    newest: Version | None = None
    newest_state: str = ""
    warnings: list[str] = field(default_factory=list)

    @property
    def has_branch(self) -> bool:
        return self.local or self.remote

    @property
    def merged(self) -> bool:
        return self.has_branch and self.ahead == 0


def channel_warnings(row: ChannelRow, live: Version | None) -> list[str]:
    out = []
    base_of = row.newest or Version.parse(row.listed_tag or "")
    if live and base_of and base_of.is_channel and base_of.base <= live:
        out.append(f"base <= live {live}")
    if row.listed:
        tag_v = Version.parse(row.listed_tag or "")
        if tag_v is None or tag_v.channel != row.id:
            out.append("listed tag invalid")
        elif row.listed_state not in (PRERELEASE, UNKNOWN):
            out.append("listed tag unpublished")
        if not row.has_branch:
            out.append("listed, no branch")
        elif row.merged:
            out.append("merged, still listed")
    return out


def build_rows(doc: dict, tags: set[str], releases: dict[str, str] | None,
               local: set[str], remote: set[str],
               counts: dict[str, tuple[int, int]]) -> list[ChannelRow]:
    """One row per channel: listed in channels.json, a channel/* branch, or a tag.

    `local` / `remote` are channel ids with a branch, `counts` maps an id to
    (ahead, behind) against main.
    """
    live = latest_live(tags, releases)
    ids = {e["id"] for e in doc["channels"]} | local | remote
    ids |= {v.channel for v in map(Version.parse, tags) if v and v.is_channel}
    rows = []
    for cid in sorted(ids):
        entry = find_channel(doc, cid)
        own = versions_of(cid, tags)
        row = ChannelRow(id=cid, local=cid in local, remote=cid in remote)
        if entry:
            row.listed = True
            row.listed_tag = entry.get("tag")
            row.listed_state = release_state(row.listed_tag, releases)
        row.ahead, row.behind = counts.get(cid, (None, None))
        if own:
            row.newest = own[-1]
            row.newest_state = release_state(row.newest.tag, releases)
        row.warnings = channel_warnings(row, live)
        rows.append(row)
    return rows


@dataclass
class LiveInfo:
    latest: Version | None  # newest full release
    newest_tag: Version | None  # newest live tag, whatever its release state
    newest_state: str
    drafts: list[str]  # every draft release, newest first


def build_live(tags: set[str], releases: dict[str, str] | None) -> LiveInfo:
    lives = sorted(v for v in map(Version.parse, tags) if v and not v.is_channel)
    newest = lives[-1] if lives else None
    drafts = []
    if releases:
        drafted = [Version.parse(t) for t, s in releases.items() if s == DRAFT]
        drafts = [v.tag for v in sorted((v for v in drafted if v), reverse=True)]
    return LiveInfo(latest=latest_live(tags, releases), newest_tag=newest,
                    newest_state=release_state(newest.tag if newest else None,
                                               releases),
                    drafts=drafts)


# --- CI build runs -----------------------------------------------------------

# What `gh run view --json` reports for a job that has not started / ended.
_NO_TIME = "0001-01-01T00:00:00Z"


@dataclass
class StepRow:
    name: str
    state: str  # queued / running / success / failure / cancelled / skipped
    seconds: int | None  # run time so far or in total; None before the start


@dataclass
class JobRow:
    name: str
    state: str
    seconds: int | None
    steps: list[StepRow] = field(default_factory=list)

    @property
    def expanded(self) -> bool:
        """Its steps are shown: while it runs, and when it failed."""
        return self.state in ("running", "failure")


@dataclass
class RunView:
    state: str  # queued / running / success / failure / cancelled
    url: str
    jobs: list[JobRow]

    @property
    def done(self) -> bool:
        return self.state not in ("queued", "running")


def _state(status: str, conclusion: str) -> str:
    if status != "completed":
        return "running" if status == "in_progress" else "queued"
    return conclusion or "failure"


def _parse_time(text: str | None):
    from datetime import datetime
    if not text or text == _NO_TIME:
        return None
    try:
        return datetime.fromisoformat(text.replace("Z", "+00:00"))
    except ValueError:
        return None


def _timed(item: dict, now) -> tuple[str, str, int | None]:
    """(name, state, seconds) of a job or step; `now` times a running one."""
    start = _parse_time(item.get("startedAt"))
    end = _parse_time(item.get("completedAt")) or (now if start else None)
    seconds = int((end - start).total_seconds()) if start and end else None
    return (str(item.get("name", "?")), _state(item.get("status", ""), item.get("conclusion", "")),
            max(seconds, 0) if seconds is not None else None)


def parse_run(data: dict, now) -> RunView:
    """`gh run view --json status,conclusion,url,jobs` as rows, each job
    with its steps; `now` (an aware datetime) times what still runs."""
    jobs = []
    for job in data.get("jobs") or []:
        steps = sorted((s for s in job.get("steps") or [] if isinstance(s, dict)),
                       key=lambda s: s.get("number", 0))
        jobs.append(JobRow(*_timed(job, now), steps=[StepRow(*_timed(s, now)) for s in steps]))
    return RunView(_state(data.get("status", ""), data.get("conclusion", "")),
                   str(data.get("url", "")), jobs)


def duration(seconds: int | None) -> str:
    return "" if seconds is None else f"{seconds // 60}:{seconds % 60:02d}"
