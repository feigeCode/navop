#!/usr/bin/env python3
"""Maintain and extract Navop's bilingual release changelog."""

from __future__ import annotations

import argparse
import datetime as dt
import os
from pathlib import Path
import re
import sys
import tempfile


INSERTION_MARKER = "<!-- NAVOP_RELEASES -->"
TAG_PATTERN = r"v[0-9]+\.[0-9]+\.[0-9]+(?:[.-][0-9A-Za-z.-]+)?"
CNB_MIRROR_LINE_RE = re.compile(
    r"国内下载：如果 GitHub 下载较慢，可从 \[CNB 镜像\]"
    r"\(https://cnb\.cool/navop-dev/navop/-/releases/tag/[^)]+\)"
    r" 下载桌面端安装包"
)
VERSION_HEADING_RE = re.compile(
    rf"^## \[(?P<tag>{TAG_PATTERN})\] - (?P<date>[0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}})[ \t]*$",
    re.MULTILINE,
)


class ChangelogError(ValueError):
    """Raised when changelog content is missing or malformed."""


def normalize_tag(value: str) -> str:
    tag = value.strip()
    if not tag.startswith("v"):
        tag = f"v{tag}"
    if re.fullmatch(TAG_PATTERN, tag) is None:
        raise ChangelogError(
            f"invalid tag {value!r}; expected vX.Y.Z or a prerelease such as vX.Y.Z-rc.1"
        )
    return tag


def validate_iso_date(value: str) -> str:
    try:
        parsed = dt.date.fromisoformat(value)
    except ValueError as error:
        raise ChangelogError(f"invalid ISO release date {value!r}") from error
    if parsed.isoformat() != value:
        raise ChangelogError(f"invalid ISO release date {value!r}")
    return value


def validate_release_notes(
    notes: str, *, require_cnb_line: bool = False
) -> None:
    if not notes.strip():
        raise ChangelogError("release notes are empty")
    chinese_sections = ("更新内容", "修复与优化")
    english_sections = ("What's New", "Fixes and Improvements")
    if not any(
        re.search(rf"^### {re.escape(section)}[ \t]*$", notes, re.MULTILINE)
        for section in chinese_sections
    ):
        raise ChangelogError("release notes are missing a Chinese content section")
    if not any(
        re.search(rf"^### {re.escape(section)}[ \t]*$", notes, re.MULTILINE)
        for section in english_sections
    ):
        raise ChangelogError("release notes are missing an English content section")
    if require_cnb_line and CNB_MIRROR_LINE_RE.search(notes) is None:
        raise ChangelogError(
            "release notes are missing the CNB mirror download line "
            "(国内下载 / CNB 镜像)"
        )


def transform_headings(markdown: str, delta: int) -> str:
    """Shift ATX headings outside fenced code blocks by one level."""

    transformed: list[str] = []
    fence_character: str | None = None
    fence_length = 0

    for line in markdown.splitlines(keepends=True):
        stripped = line.lstrip()
        fence = re.match(r"(`{3,}|~{3,})", stripped)
        if fence:
            marker = fence.group(1)
            if fence_character is None:
                fence_character = marker[0]
                fence_length = len(marker)
            elif marker[0] == fence_character and len(marker) >= fence_length:
                fence_character = None
                fence_length = 0
            transformed.append(line)
            continue

        if fence_character is None:
            content = line.rstrip("\r\n")
            line_ending = line[len(content) :]
            if delta < 0:
                match = re.match(r"^(#{3,6})([ \t]+.*)$", content)
                if match:
                    line = f"{match.group(1)[1:]}{match.group(2)}{line_ending}"
            elif delta > 0:
                match = re.match(r"^(#{2,5})([ \t]+.*)$", content)
                if match:
                    line = f"#{match.group(1)}{match.group(2)}{line_ending}"

        transformed.append(line)

    return "".join(transformed)


def mask_fenced_blocks(markdown: str) -> str:
    """Replace fenced block contents with spaces while preserving offsets."""

    masked: list[str] = []
    fence_character: str | None = None
    fence_length = 0

    for line in markdown.splitlines(keepends=True):
        stripped = line.lstrip()
        fence = re.match(r"(`{3,}|~{3,})", stripped)
        inside_fence = fence_character is not None

        if fence and not inside_fence:
            marker = fence.group(1)
            fence_character = marker[0]
            fence_length = len(marker)
            inside_fence = True
        elif (
            fence
            and inside_fence
            and fence.group(1)[0] == fence_character
            and len(fence.group(1)) >= fence_length
        ):
            fence_character = None
            fence_length = 0

        if inside_fence:
            masked.append(
                "".join(character if character in "\r\n" else " " for character in line)
            )
        else:
            masked.append(line)

    return "".join(masked)


def version_matches(changelog: str) -> list[re.Match[str]]:
    matches = list(VERSION_HEADING_RE.finditer(mask_fenced_blocks(changelog)))
    tags = [match.group("tag") for match in matches]
    duplicates = sorted({tag for tag in tags if tags.count(tag) > 1})
    if duplicates:
        raise ChangelogError(
            f"duplicate changelog entries found for: {', '.join(duplicates)}"
        )
    for match in matches:
        validate_iso_date(match.group("date"))
    return matches


def extract_release_notes(changelog: str, tag: str) -> str:
    normalized_tag = normalize_tag(tag)
    matches = version_matches(changelog)

    for index, match in enumerate(matches):
        if match.group("tag") != normalized_tag:
            continue
        end = matches[index + 1].start() if index + 1 < len(matches) else len(changelog)
        entry_body = changelog[match.end() : end].strip()
        if not entry_body:
            raise ChangelogError(f"changelog entry for {normalized_tag} is empty")
        notes = transform_headings(entry_body, -1).strip() + "\n"
        validate_release_notes(notes)
        return notes

    raise ChangelogError(f"CHANGELOG.md has no entry for {normalized_tag}")


def atomic_write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    file_descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", dir=path.parent
    )
    try:
        with os.fdopen(file_descriptor, "w", encoding="utf-8", newline="\n") as file:
            file.write(content)
        os.replace(temporary_name, path)
    except BaseException:
        try:
            os.unlink(temporary_name)
        except FileNotFoundError:
            pass
        raise


def upsert_release(
    changelog: str, tag: str, release_date: str, release_notes: str
) -> str:
    normalized_tag = normalize_tag(tag)
    validate_iso_date(release_date)
    normalized_notes = release_notes.strip() + "\n"
    validate_release_notes(normalized_notes, require_cnb_line=True)

    if INSERTION_MARKER not in changelog:
        raise ChangelogError(
            f"CHANGELOG.md is missing insertion marker {INSERTION_MARKER}"
        )
    if changelog.count(INSERTION_MARKER) != 1:
        raise ChangelogError(
            f"CHANGELOG.md must contain exactly one {INSERTION_MARKER} marker"
        )

    entry_body = transform_headings(normalized_notes, 1).strip()
    entry = f"## [{normalized_tag}] - {release_date}\n\n{entry_body}\n"
    matches = version_matches(changelog)

    for index, match in enumerate(matches):
        if match.group("tag") != normalized_tag:
            continue
        end = matches[index + 1].start() if index + 1 < len(matches) else len(changelog)
        prefix = changelog[: match.start()].rstrip()
        suffix = changelog[end:].lstrip("\n")
        result = f"{prefix}\n\n{entry}"
        if suffix:
            result += f"\n{suffix}"
        return result.rstrip() + "\n"

    marker_end = changelog.index(INSERTION_MARKER) + len(INSERTION_MARKER)
    prefix = changelog[:marker_end].rstrip()
    suffix = changelog[marker_end:].lstrip("\n")
    result = f"{prefix}\n\n{entry}"
    if suffix:
        result += f"\n{suffix}"
    return result.rstrip() + "\n"


def read_utf8(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError as error:
        raise ChangelogError(f"file not found: {path}") from error


def command_extract(arguments: argparse.Namespace) -> None:
    changelog = read_utf8(arguments.changelog)
    notes = extract_release_notes(changelog, arguments.tag)
    if str(arguments.output) == "-":
        sys.stdout.write(notes)
    else:
        atomic_write(arguments.output, notes)


def command_upsert(arguments: argparse.Namespace) -> None:
    changelog = read_utf8(arguments.changelog)
    notes = read_utf8(arguments.notes_file)
    updated = upsert_release(changelog, arguments.tag, arguments.date, notes)
    atomic_write(arguments.changelog, updated)


def command_validate(arguments: argparse.Namespace) -> None:
    changelog = read_utf8(arguments.changelog)
    notes = extract_release_notes(changelog, arguments.tag)
    validate_release_notes(notes, require_cnb_line=True)
    print(f"Validated changelog entry: {normalize_tag(arguments.tag)}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Maintain Navop's bilingual CHANGELOG.md release entries."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    extract_parser = subparsers.add_parser(
        "extract", help="extract one changelog entry as a GitHub Release body"
    )
    extract_parser.add_argument("--tag", required=True)
    extract_parser.add_argument(
        "--changelog", type=Path, default=Path("CHANGELOG.md")
    )
    extract_parser.add_argument("--output", type=Path, default=Path("-"))
    extract_parser.set_defaults(handler=command_extract)

    upsert_parser = subparsers.add_parser(
        "upsert", help="insert or replace a changelog entry from release notes"
    )
    upsert_parser.add_argument("--tag", required=True)
    upsert_parser.add_argument("--date", required=True)
    upsert_parser.add_argument("--notes-file", type=Path, required=True)
    upsert_parser.add_argument(
        "--changelog", type=Path, default=Path("CHANGELOG.md")
    )
    upsert_parser.set_defaults(handler=command_upsert)

    validate_parser = subparsers.add_parser(
        "validate", help="validate and round-trip one changelog entry"
    )
    validate_parser.add_argument("--tag", required=True)
    validate_parser.add_argument(
        "--changelog", type=Path, default=Path("CHANGELOG.md")
    )
    validate_parser.set_defaults(handler=command_validate)

    return parser


def main() -> int:
    parser = build_parser()
    arguments = parser.parse_args()
    try:
        arguments.handler(arguments)
    except ChangelogError as error:
        parser.error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
