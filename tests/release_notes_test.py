"""Regression tests for scripts/release_notes.py (run: python3 -m unittest tests/release_notes_test.py)."""
import importlib.util
import pathlib
import unittest

SPEC = importlib.util.spec_from_file_location(
    "release_notes", pathlib.Path(__file__).resolve().parents[1] / "scripts" / "release_notes.py"
)
MOD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MOD)


class MentionTests(unittest.TestCase):
    def test_bare_mention_is_wrapped(self):
        self.assertEqual(MOD.neutralize_mentions("Đọc trước cho @AI"), "Đọc trước cho `@AI`")

    def test_code_span_and_email_untouched(self):
        self.assertEqual(MOD.neutralize_mentions("Trợ lý `@AI` và a@b.com"), "Trợ lý `@AI` và a@b.com")

    def test_release_body_has_no_bare_mention(self):
        changelog = "## [1.0.0] - 2026-01-01\n\n### Added\n- **Đọc trước ngữ cảnh cho @AI:** chi tiết\n"
        body = MOD.build("1.0.0", changelog, "owner/repo")
        self.assertIn("- Đọc trước ngữ cảnh cho `@AI`", body)


if __name__ == "__main__":
    unittest.main()
