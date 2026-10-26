import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from preferences import _herdr_directories, client_preferences_path, migrate_legacy_preferences


class LegacyPreferencesTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.config = self.root / "config/herdr"
        self.state = self.root / "state/herdr"
        self.source = self.config / "session.json"
        self.target = client_preferences_path(self.config, self.state)
        self.backups = self.root / "backups"
        environment = {
            "HOME": str(self.root / "home"),
            "XDG_CONFIG_HOME": str(self.root / "config"),
            "XDG_STATE_HOME": str(self.root / "state"),
        }
        self.environment = patch.dict(os.environ, environment, clear=True)
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def write_json(self, path, value):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value) + "\n", encoding="utf-8")

    def test_hash_matches_default_release_client(self):
        path = client_preferences_path(
            Path("/home/example/.config/herdr"),
            Path("/home/example/.local/state/herdr"),
        )
        self.assertEqual(path.name, "local-69727b65fd075609.json")

    def test_default_paths_match_home_fallback(self):
        with patch.dict(os.environ, {"HOME": str(self.root)}, clear=True):
            self.assertEqual(_herdr_directories(), (
                self.root / ".config/herdr", self.root / ".local/state/herdr",
            ))

    def test_migrates_only_legacy_chrome_fields(self):
        snapshot = {
            "sidebar_width": 30,
            "sidebar_section_split": 0.1,
            "collapsed_space_keys": ["/repo"],
            "workspaces": [{"irrelevant": "not copied"}],
        }
        self.write_json(self.source, snapshot)
        receipt = migrate_legacy_preferences(self.backups)
        self.assertEqual(receipt["status"], "migrated")
        self.assertIsNone(receipt["backup"])
        self.assertEqual(json.loads(self.target.read_text()), {
            "sidebar_width": 30,
            "sidebar_section_split": 0.1,
            "collapsed_groups": ["/repo"],
        })
        self.assertEqual(json.loads(self.source.read_text()), snapshot)
        self.assertFalse(self.backups.exists())

    def test_merges_missing_fields_and_preserves_existing_preferences(self):
        self.write_json(self.source, {
            "sidebar_width": 30,
            "sidebar_section_split": 0.1,
            "collapsed_space_keys": [],
        })
        self.write_json(self.target, {
            "sidebar_width": 24,
            "sidebar_collapsed": True,
            "agent_panel_sort": "priority",
            "remote_collapsed_groups": [{"profile_id": "remote", "collapsed_groups": ["x"]}],
        })
        original = self.target.read_bytes()
        receipt = migrate_legacy_preferences(self.backups)
        self.assertEqual(receipt["migrated_fields"], ["collapsed_groups", "sidebar_section_split"])
        self.assertEqual(Path(receipt["backup"]).read_bytes(), original)
        preferences = json.loads(self.target.read_text())
        self.assertEqual(preferences["sidebar_width"], 24)
        self.assertTrue(preferences["sidebar_collapsed"])
        self.assertEqual(preferences["agent_panel_sort"], "priority")
        self.assertEqual(preferences["remote_collapsed_groups"], [
            {"profile_id": "remote", "collapsed_groups": ["x"]},
        ])
        self.assertEqual(preferences["sidebar_section_split"], 0.1)
        self.assertEqual(preferences["collapsed_groups"], [])
        self.assertEqual(list(self.target.parent.glob("*.tmp-*")), [])
        unchanged = self.target.read_bytes()
        repeated = migrate_legacy_preferences(self.backups)
        self.assertEqual(repeated["status"], "no_changes")
        self.assertEqual(self.target.read_bytes(), unchanged)
        self.assertEqual(len(list(self.backups.iterdir())), 1)

    def test_missing_source_is_a_no_op(self):
        self.write_json(self.target, {"sidebar_width": 24})
        original = self.target.read_bytes()
        receipt = migrate_legacy_preferences(self.backups)
        self.assertEqual(receipt["status"], "source_missing")
        self.assertEqual(self.target.read_bytes(), original)
        self.assertFalse(self.backups.exists())

    def test_invalid_target_is_not_replaced(self):
        self.write_json(self.source, {"sidebar_width": 30})
        self.target.parent.mkdir(parents=True)
        self.target.write_text("not JSON", encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "Invalid client preferences"):
            migrate_legacy_preferences(self.backups)
        self.assertEqual(self.target.read_text(), "not JSON")
        self.assertFalse(self.backups.exists())

    def test_failed_atomic_replace_keeps_original_and_backup(self):
        self.write_json(self.source, {"sidebar_width": 30})
        self.write_json(self.target, {"agent_panel_sort": "priority"})
        original = self.target.read_bytes()
        with patch("preferences.os.replace", side_effect=OSError("replace failed")):
            with self.assertRaises(OSError):
                migrate_legacy_preferences(self.backups)
        self.assertEqual(self.target.read_bytes(), original)
        backups = list(self.backups.iterdir())
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_bytes(), original)
        self.assertEqual(list(self.target.parent.glob("*.tmp-*")), [])


if __name__ == "__main__":
    unittest.main()
