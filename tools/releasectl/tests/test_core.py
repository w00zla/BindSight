"""Unit tests for the pure release logic (no git, no network)."""

import json
import unittest

from releasectl import core
from releasectl.core import Version as V


def v(text):
    parsed = V.parse(text)
    assert parsed is not None, text
    return parsed


class VersionParse(unittest.TestCase):
    def test_live_and_channel(self):
        self.assertEqual(v("0.17.0"), V(0, 17, 0))
        self.assertEqual(v("v0.18.0-joysticks.2"), V(0, 18, 0, "joysticks", 2))
        self.assertEqual(str(v("v1.2.3-a-b.10")), "1.2.3-a-b.10")
        self.assertEqual(v("1.2.3-x.1").tag, "v1.2.3-x.1")

    def test_rejects(self):
        for bad in ["", "1.2", "1.2.3.4", "01.2.3", "1.2.3-", "1.2.3-x",
                    "1.2.3-x.01", "1.2.3-X.1", "1.2.3-1x.1", "1.2.3-stable.1",
                    "1.2.3-x.y.1", "1.2.3-" + "a" * 33 + ".1", "prerelease-version",
                    " 1.2.3", "1.2.3 "]:
            self.assertIsNone(V.parse(bad), bad)

    def test_id_length_limit(self):
        self.assertIsNotNone(V.parse("1.2.3-" + "a" * 32 + ".1"))

    def test_n_zero_is_valid_shape(self):
        self.assertEqual(v("1.0.0-x.0").n, 0)


class VersionOrder(unittest.TestCase):
    def test_prerelease_before_release(self):
        self.assertLess(v("0.17.0-x.3"), v("0.17.0"))
        self.assertLess(v("0.17.0"), v("0.18.0-x.1"))

    def test_numeric_n(self):
        self.assertLess(v("1.0.0-x.2"), v("1.0.0-x.10"))

    def test_core_numeric(self):
        self.assertLess(v("0.9.0"), v("0.10.0"))
        self.assertLess(v("0.10.9"), v("0.10.10"))

    def test_channels_ascii(self):
        self.assertLess(v("1.0.0-a.9"), v("1.0.0-b.1"))

    def test_sorting(self):
        tags = ["0.2.0", "0.17.0", "0.17.0-x.1", "0.10.0", "0.17.0-x.12", "0.17.0-x.2"]
        self.assertEqual([str(x) for x in sorted(map(v, tags))],
                         ["0.2.0", "0.10.0", "0.17.0-x.1", "0.17.0-x.2",
                          "0.17.0-x.12", "0.17.0"])


class ChannelId(unittest.TestCase):
    def test_valid(self):
        for ok in ["x", "joysticks", "a1", "a-b", "a" * 32]:
            self.assertIsNone(core.validate_channel_id(ok), ok)

    def test_invalid(self):
        for bad in ["", "1a", "-a", "A", "a_b", "a.b", "a" * 33, "stable", "a b"]:
            self.assertIsNotNone(core.validate_channel_id(bad), bad)


class NextVersions(unittest.TestCase):
    def test_live_proposal_follows_the_higher_of_release_and_tag(self):
        # 0.16.2 / 0.16.3 were tagged (pre-releases of the old model), 0.16.1 is live.
        self.assertEqual(core.live_proposal(v("0.16.1"), v("0.16.3")), v("0.17.0"))
        self.assertEqual(core.live_proposal(v("0.17.0"), None), v("0.18.0"))
        self.assertEqual(core.live_proposal(None, v("0.3.2")), v("0.4.0"))
        self.assertEqual(core.live_proposal(None, None), v("0.1.0"))

    def test_first_channel_version_is_next_minor_of_live(self):
        self.assertEqual(core.next_channel_version("x", [], v("0.16.1")), v("0.17.0-x.1"))

    def test_next_n(self):
        self.assertEqual(core.next_channel_version("x", [v("0.17.0-x.1"), v("0.17.0-x.2")], v("0.16.1")),
                         v("0.17.0-x.3"))

    def test_other_channels_ignored(self):
        self.assertEqual(core.next_channel_version("x", [v("0.19.0-y.4")], v("0.16.1")),
                         v("0.17.0-x.1"))

    def test_base_released_moves_to_next_minor(self):
        self.assertEqual(core.next_channel_version("x", [v("0.17.0-x.3")], v("0.17.0")),
                         v("0.18.0-x.1"))
        self.assertEqual(core.next_channel_version("x", [v("0.17.0-x.3")], v("0.17.2")),
                         v("0.18.0-x.1"))

    def test_no_live_at_all(self):
        self.assertEqual(core.next_channel_version("x", [], None), v("0.1.0-x.1"))
        self.assertEqual(core.next_channel_version("x", [v("0.3.0-x.1")], None), v("0.3.0-x.2"))

    def test_next_live(self):
        self.assertEqual(core.next_live(v("0.17.0"), v("0.18.0-x.2")), v("0.18.0"))
        self.assertEqual(core.next_live(v("0.18.0"), v("0.18.0-x.2")), v("0.19.0"))
        self.assertEqual(core.next_live(v("0.17.0"), None), v("0.18.0"))


class Bump(unittest.TestCase):
    TOML = ('[package]\nname = "bindsight"\nversion = "0.16.3"\n\n'
            '[dependencies]\nfoo = { version = "1.0" }\n[x]\nversion = "0.16.3"\n')
    LOCK = ('[[package]]\nname = "aaa"\nversion = "0.16.3"\n\n'
            '[[package]]\nname = "bindsight"\nversion = "0.16.3"\n')
    PKG = ('{\n  "name": "bindsight",\n  "version": "0.16.3",\n'
           '  "dependencies": {\n    "x": {\n      "version": "0.16.3",\n'
           '      "y": 1\n    }\n  }\n}\n')

    def texts(self):
        return {"src-tauri/Cargo.toml": self.TOML, "src-tauri/Cargo.lock": self.LOCK,
                "package.json": self.PKG}

    def test_bump_sets_only_our_lines(self):
        out = core.bump_texts(self.texts(), "0.17.0-x.1")
        self.assertEqual(out["src-tauri/Cargo.toml"],
                         self.TOML.replace('version = "0.16.3"\n\n',
                                           'version = "0.17.0-x.1"\n\n', 1))
        self.assertIn('name = "aaa"\nversion = "0.16.3"', out["src-tauri/Cargo.lock"])
        self.assertIn('name = "bindsight"\nversion = "0.17.0-x.1"', out["src-tauri/Cargo.lock"])
        self.assertIn('  "version": "0.17.0-x.1",', out["package.json"])
        self.assertIn('      "version": "0.16.3",', out["package.json"])
        # Only the version line changed.
        self.assertEqual(out["package.json"].count("0.16.3"), 1)

    def test_crlf_lock(self):
        texts = self.texts()
        texts["src-tauri/Cargo.lock"] = self.LOCK.replace("\n", "\r\n")
        # Cargo.toml / package.json with LF, the lock with CRLF: '$' does not
        # match before '\r', so the lock stays unchanged and the bump refuses.
        with self.assertRaises(ValueError):
            core.bump_texts(texts, "0.17.0")

    def test_missing_line_raises(self):
        texts = self.texts()
        texts["package.json"] = texts["package.json"].replace('"version": "0.16.3"', '"v": 1')
        with self.assertRaises(ValueError):
            core.bump_texts(texts, "0.17.0")

    def test_same_version_raises(self):
        with self.assertRaises(ValueError):
            core.bump_texts(self.texts(), "0.16.3")

    def test_no_version_in_toml(self):
        texts = self.texts()
        texts["src-tauri/Cargo.toml"] = "[package]\n"
        with self.assertRaises(ValueError):
            core.bump_texts(texts, "0.17.0")

    def test_current_version(self):
        self.assertEqual(core.cargo_toml_version(self.TOML), "0.16.3")

    def test_branch_guard(self):
        self.assertIsNone(core.branch_guard(v("0.17.0"), "main"))
        self.assertIsNotNone(core.branch_guard(v("0.17.0"), "channel/x"))
        self.assertIsNone(core.branch_guard(v("0.17.0-x.1"), "channel/x"))
        self.assertIsNotNone(core.branch_guard(v("0.17.0-x.1"), "channel/y"))
        self.assertIsNotNone(core.branch_guard(v("0.17.0-x.1"), "main"))

    def test_messages(self):
        self.assertEqual(core.commit_message(v("0.17.0")), "Bump version to 0.17.0")
        self.assertEqual(core.tag_message(v("0.17.0-x.1")), "BindSight 0.17.0-x.1")


class ChannelsJson(unittest.TestCase):
    TEXT = ('{\n  "channels": [\n    {\n      "id": "joysticks",\n'
            '      "tag": "v0.18.0-joysticks.2"\n'
            '    },\n    {\n      "id": "other",\n      "tag": "v0.18.0-other.1"\n    }\n  ]\n}\n')

    def test_missing_file(self):
        self.assertEqual(core.load_channels(None), {"channels": []})

    def test_roundtrip_is_stable(self):
        self.assertEqual(core.dump_channels(core.load_channels(self.TEXT)), self.TEXT)

    def test_key_order_and_unknown_keys(self):
        doc = {"channels": [{"tag": "v1.0.0-a.1", "extra": 1, "id": "a"}]}
        out = core.dump_channels(doc)
        entry = json.loads(out)["channels"][0]
        self.assertEqual(list(entry), ["id", "tag", "extra"])
        self.assertTrue(out.endswith("}\n"))

    def test_remove(self):
        doc = core.load_channels(self.TEXT)
        out = core.remove_channel(doc, "joysticks")
        self.assertEqual([e["id"] for e in out["channels"]], ["other"])
        self.assertEqual(len(doc["channels"]), 2)  # input untouched
        with self.assertRaises(KeyError):
            core.remove_channel(doc, "nope")

    def test_remove_last_keeps_empty_list(self):
        doc = core.load_channels('{"channels": [{"id": "a", "tag": "v1.0.0-a.1"}]}')
        self.assertEqual(core.dump_channels(core.remove_channel(doc, "a")),
                         '{\n  "channels": []\n}\n')

    def test_invalid(self):
        for bad in ["[]", '{"channels": {}}', '{"channels": [{"tag": "x"}]}',
                    '{"channels": [1]}', "{"]:
            with self.assertRaises(ValueError, msg=bad):
                core.load_channels(bad)

    def test_no_channels_key(self):
        self.assertEqual(core.load_channels("{}"), {"channels": []})


REL = {"v0.16.1": core.RELEASE, "v0.16.2": core.PRERELEASE,
       "v0.17.0": core.RELEASE, "v0.18.0-j.1": core.PRERELEASE,
       "v0.18.0-j.2": core.DRAFT, "v0.19.0": core.DRAFT}


class Overview(unittest.TestCase):
    def rows(self, doc, tags=None, releases=REL, local=(), remote=(),
             counts=None):
        tags = set(tags if tags is not None else REL)
        return {r.id: r for r in core.build_rows(
            doc, tags, releases, set(local), set(remote), counts or {})}

    def test_latest_live(self):
        self.assertEqual(core.latest_live(set(REL), REL), v("0.17.0"))
        self.assertIsNone(core.latest_live(set(REL), None))

    def test_live_info(self):
        info = core.build_live(set(REL), REL)
        self.assertEqual(info.latest, v("0.17.0"))
        self.assertEqual(info.newest_tag, v("0.19.0"))
        self.assertEqual(info.newest_state, core.DRAFT)
        self.assertEqual(info.drafts, ["v0.19.0", "v0.18.0-j.2"])

    def test_union_of_sources(self):
        doc = {"channels": [{"id": "listed", "tag": "v0.18.0-listed.1"}]}
        rows = self.rows(doc, tags=set(REL) | {"v0.18.0-tagonly.1"},
                         local={"loc"}, remote={"rem"})
        self.assertEqual(set(rows), {"listed", "j", "tagonly", "loc", "rem"})

    def test_healthy_channel(self):
        doc = {"channels": [{"id": "j", "tag": "v0.18.0-j.1"}]}
        row = self.rows(doc, local={"j"}, counts={"j": (3, 0)})["j"]
        self.assertEqual(row.newest, v("0.18.0-j.2"))
        self.assertEqual(row.newest_state, core.DRAFT)
        self.assertEqual(row.listed_state, core.PRERELEASE)
        self.assertEqual(row.warnings, [])

    def test_base_not_above_live(self):
        rel = dict(REL, **{"v0.17.0-old.3": core.PRERELEASE})
        row = self.rows({"channels": []}, tags=set(rel), releases=rel, remote={"old"},
                        counts={"old": (1, 5)})["old"]
        self.assertEqual(row.warnings, ["base <= live 0.17.0"])

    def test_listed_unpublished_and_no_branch(self):
        doc = {"channels": [{"id": "j", "tag": "v0.18.0-j.2"}]}
        row = self.rows(doc)["j"]
        self.assertIn("listed tag unpublished", row.warnings)
        self.assertIn("listed, no branch", row.warnings)

    def test_listed_tag_missing_release(self):
        doc = {"channels": [{"id": "j", "tag": "v0.18.0-j.9"}]}
        row = self.rows(doc, local={"j"}, counts={"j": (1, 0)})["j"]
        self.assertEqual(row.listed_state, core.MISSING)
        self.assertEqual(row.warnings, ["listed tag unpublished"])

    def test_merged_still_listed(self):
        doc = {"channels": [{"id": "j", "tag": "v0.18.0-j.1"}]}
        row = self.rows(doc, local={"j"}, counts={"j": (0, 4)})["j"]
        self.assertEqual(row.warnings, ["merged, still listed"])

    def test_listed_tag_invalid(self):
        for tag in [None, "v0.18.0-other.1", "garbage"]:
            doc = {"channels": [{"id": "j", "tag": tag}]}
            row = self.rows(doc, local={"j"}, counts={"j": (1, 0)})["j"]
            self.assertIn("listed tag invalid", row.warnings, tag)

    def test_gh_unreachable_no_publication_warnings(self):
        doc = {"channels": [{"id": "j", "tag": "v0.18.0-j.1"}]}
        row = self.rows(doc, releases=None, local={"j"}, counts={"j": (1, 0)})["j"]
        self.assertEqual(row.listed_state, core.UNKNOWN)
        self.assertEqual(row.warnings, [])


if __name__ == "__main__":
    unittest.main()
