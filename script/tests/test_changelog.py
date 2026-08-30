from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT_PATH = Path(__file__).resolve().parents[1] / "changelog.py"
SPEC = importlib.util.spec_from_file_location("navop_changelog", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
changelog = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(changelog)


HEADER = """\
# Changelog

Navop release notes.

<!-- NAVOP_RELEASES -->
"""

NOTES = """\
### 更新内容

- 新功能

### 修复与优化

- 修复问题

国内下载：如果 GitHub 下载较慢，可从 [CNB 镜像](https://cnb.cool/navop-dev/navop/-/releases/tag/v1.2.0) 下载桌面端安装包

---

### What's New

- New feature

### Fixes and Improvements

- Fixed a bug

**Full Changelog**: https://github.com/feigeCode/navop/compare/v1.1.0...v1.2.0
"""


class ChangelogTests(unittest.TestCase):
    def test_extract_seeded_repository_entry(self) -> None:
        repository_root = SCRIPT_PATH.parents[1]
        text = (repository_root / "CHANGELOG.md").read_text(encoding="utf-8")

        notes = changelog.extract_release_notes(text, "v0.10.0")

        self.assertIn("### 更新内容", notes)
        self.assertIn("### What's New", notes)
        self.assertIn(
            "compare/v0.9.8...v0.10.0",
            notes,
        )

    def test_upsert_preserves_older_entries_and_round_trips(self) -> None:
        older = changelog.upsert_release(HEADER, "v1.1.0", "2026-07-01", NOTES)

        updated = changelog.upsert_release(older, "v1.2.0", "2026-08-01", NOTES)

        self.assertLess(updated.index("[v1.2.0]"), updated.index("[v1.1.0]"))
        self.assertEqual(
            changelog.extract_release_notes(updated, "v1.2.0").strip(),
            NOTES.strip(),
        )
        self.assertIn("## [v1.1.0] - 2026-07-01", updated)

    def test_upsert_is_idempotent_for_existing_version(self) -> None:
        first = changelog.upsert_release(HEADER, "1.2.0", "2026-08-01", NOTES)
        second = changelog.upsert_release(
            first, "v1.2.0", "2026-08-01", NOTES
        )

        self.assertEqual(first, second)
        self.assertEqual(second.count("## [v1.2.0]"), 1)

    def test_heading_shift_ignores_fenced_code(self) -> None:
        notes = NOTES + "\n```markdown\n## unchanged\n### unchanged\n```\n"

        updated = changelog.upsert_release(HEADER, "v1.2.0", "2026-08-01", notes)
        extracted = changelog.extract_release_notes(updated, "v1.2.0")

        self.assertIn("```markdown\n## unchanged\n### unchanged\n```", extracted)

    def test_version_heading_inside_fence_is_not_an_entry_boundary(self) -> None:
        notes = (
            NOTES
            + "\n```markdown\n"
            + "## [v9.9.9] - 2099-09-09\n"
            + "example only\n"
            + "```\n"
        )

        updated = changelog.upsert_release(HEADER, "v1.2.0", "2026-08-01", notes)
        extracted = changelog.extract_release_notes(updated, "v1.2.0")

        self.assertIn("## [v9.9.9] - 2099-09-09", extracted)
        with self.assertRaisesRegex(changelog.ChangelogError, "v9.9.9"):
            changelog.extract_release_notes(updated, "v9.9.9")

    def test_missing_bilingual_section_fails(self) -> None:
        with self.assertRaisesRegex(changelog.ChangelogError, "English content section"):
            changelog.upsert_release(
                HEADER,
                "v1.2.0",
                "2026-08-01",
                "### 更新内容\n\n- 只有中文\n",
            )

    def test_upsert_requires_cnb_mirror_line(self) -> None:
        notes_without_cnb = """\
### 更新内容

- 新功能

### 修复与优化

- 修复问题

---

### What's New

- New feature

### Fixes and Improvements

- Fixed a bug

**Full Changelog**: https://github.com/feigeCode/navop/compare/v1.1.0...v1.2.0
"""
        with self.assertRaisesRegex(
            changelog.ChangelogError, "CNB mirror download line"
        ):
            changelog.upsert_release(HEADER, "v1.2.0", "2026-08-01", notes_without_cnb)

    def test_validate_requires_cnb_mirror_line(self) -> None:
        notes_without_cnb = """\
### 更新内容

- 新功能

### What's New

- New feature

**Full Changelog**: https://github.com/feigeCode/navop/compare/v1.1.0...v1.2.0
"""
        with self.assertRaisesRegex(
            changelog.ChangelogError, "CNB mirror download line"
        ):
            changelog.validate_release_notes(notes_without_cnb, require_cnb_line=True)

    def test_extract_stays_lenient_for_legacy_entries_without_cnb_line(self) -> None:
        legacy = """\
## [v0.9.9] - 2026-06-01

#### 更新内容

- 旧功能

---

#### What's New

- Legacy feature

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.9.8...v0.9.9
"""
        full = HEADER + "\n\n" + legacy
        notes = changelog.extract_release_notes(full, "v0.9.9")
        self.assertIn("### 更新内容", notes)
        self.assertIn("Legacy feature", notes)

    def test_missing_target_entry_fails(self) -> None:
        with self.assertRaisesRegex(changelog.ChangelogError, "v9.9.9"):
            changelog.extract_release_notes(HEADER, "v9.9.9")

    def test_prerelease_tag_round_trip(self) -> None:
        updated = changelog.upsert_release(
            HEADER, "v1.2.0-rc.1", "2026-08-01", NOTES
        )

        self.assertIn("## [v1.2.0-rc.1] - 2026-08-01", updated)
        self.assertEqual(
            changelog.extract_release_notes(updated, "v1.2.0-rc.1").strip(),
            NOTES.strip(),
        )

    def test_atomic_write_creates_output(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "nested" / "notes.md"
            changelog.atomic_write(output, NOTES)
            self.assertEqual(output.read_text(encoding="utf-8"), NOTES)


if __name__ == "__main__":
    unittest.main()
