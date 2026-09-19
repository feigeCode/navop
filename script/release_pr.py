#!/usr/bin/env python3
"""Classify and verify `dev` -> `main` release pull requests.

A release pull request carries only the bilingual `CHANGELOG.md` entry and the
version bump, so it does not need the full platform test matrix. `check`
decides whether a pull request qualifies for that fast path; when it does, the
release metadata is validated here instead of by a Rust build. `ci-gate` still
blocks the merge whenever the entry is missing or malformed.

Usage (CI):
    BASE_SHA=<sha> HEAD_SHA=<sha> GITHUB_OUTPUT=<file> \
        python3 script/release_pr.py check
"""

from __future__ import annotations

import argparse
import importlib.util
import os
import re
import subprocess
import sys
from pathlib import Path

ALLOWED_FILES = ("CHANGELOG.md", "main/Cargo.toml", "Cargo.lock")
VERSION_PATHS = ("main/Cargo.toml", "Cargo.lock")
CHANGELOG_PATH = "CHANGELOG.md"
MANIFEST_PATH = "main/Cargo.toml"
LOCK_PATH = "Cargo.lock"
VERSION_LINE_RE = re.compile(r'^[+-][ \t]*version[ \t]*=[ \t]*"')
PACKAGE_SECTION_RE = re.compile(r"^\[package\]$")
SECTION_RE = re.compile(r"^\[")
NAME_RE = re.compile(r'^name[ \t]*=[ \t]*"([^"]+)"')
VERSION_RE = re.compile(r'^version[ \t]*=[ \t]*"([^"]+)"')


class ReleasePrError(Exception):
    """Raised when a release pull request cannot be classified or is invalid."""


def run_git(arguments: list[str], cwd: Path) -> str:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise ReleasePrError(
            f"git {' '.join(arguments)} failed: {completed.stderr.strip()}"
        )
    return completed.stdout


def changed_files(base: str, head: str, cwd: Path) -> list[str]:
    output = run_git(["diff", "--name-only", base, head], cwd)
    return [line.strip() for line in output.splitlines() if line.strip()]


def version_paths_diff(base: str, head: str, cwd: Path) -> str:
    return run_git(["diff", base, head, "--", *VERSION_PATHS], cwd)


def only_version_lines_changed(diff: str) -> bool:
    """True when every added/removed line is a `version = "..."` assignment."""

    for line in diff.splitlines():
        if not line.startswith(("+", "-")):
            continue
        if line.startswith(("+++", "---")):
            continue
        if VERSION_LINE_RE.match(line) is None:
            return False
    return True


def classify(files: list[str], diff: str) -> tuple[bool, str]:
    """Return whether the pull request is a release-only change and why."""

    foreign = [path for path in files if path not in ALLOWED_FILES]
    if foreign:
        return False, f"code or configuration changed: {', '.join(sorted(foreign))}"
    if not files:
        return False, "no file changed"
    if not only_version_lines_changed(diff):
        return False, "Cargo.toml / Cargo.lock changed beyond the version bump"
    return True, "only CHANGELOG.md and the version bump changed"


def manifest_version(manifest: str) -> str:
    inside_package = False
    for line in manifest.splitlines():
        if PACKAGE_SECTION_RE.match(line):
            inside_package = True
            continue
        if SECTION_RE.match(line):
            inside_package = False
            continue
        if not inside_package:
            continue
        match = VERSION_RE.match(line)
        if match:
            return match.group(1)
    raise ReleasePrError(f"{MANIFEST_PATH} has no [package] version")


def lock_main_version(lock: str) -> str:
    current_name: str | None = None
    for line in lock.splitlines():
        if line == "[[package]]":
            current_name = None
            continue
        name_match = NAME_RE.match(line)
        if name_match:
            current_name = name_match.group(1)
            continue
        if current_name != "main":
            continue
        version_match = VERSION_RE.match(line)
        if version_match:
            return version_match.group(1)
    raise ReleasePrError(f'{LOCK_PATH} has no version for the "main" package')


def load_changelog_module():
    script_path = Path(__file__).resolve().parent / "changelog.py"
    spec = importlib.util.spec_from_file_location("navop_changelog", script_path)
    if spec is None or spec.loader is None:
        raise ReleasePrError(f"unable to load {script_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def validate_changelog_entry(changelog: str, version: str) -> None:
    module = load_changelog_module()
    tag = f"v{version}"
    try:
        notes = module.extract_release_notes(changelog, tag)
        module.validate_release_notes(notes, require_cnb_line=True)
    except module.ChangelogError as error:
        raise ReleasePrError(str(error)) from error


def read(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError as error:
        raise ReleasePrError(f"file not found: {path}") from error


def verify_release_metadata(root: Path, changelog: str) -> str:
    """Validate the release entry and the version pair; return the version."""

    version = manifest_version(read(root / MANIFEST_PATH))
    locked = lock_main_version(read(root / LOCK_PATH))
    if version != locked:
        raise ReleasePrError(
            f"{MANIFEST_PATH} is {version} but {LOCK_PATH} pins main at {locked}"
        )
    validate_changelog_entry(changelog, version)
    return version


def write_github_output(path: Path, values: dict[str, str]) -> None:
    with path.open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            handle.write(f"{key}={value}\n")


def command_check(arguments: argparse.Namespace) -> None:
    base = arguments.base or os.environ.get("BASE_SHA", "")
    head = arguments.head or os.environ.get("HEAD_SHA", "")
    if not base or not head:
        raise ReleasePrError("BASE_SHA and HEAD_SHA are required")

    root = arguments.root.resolve()
    try:
        files = changed_files(base, head, root)
        diff = version_paths_diff(base, head, root)
    except ReleasePrError as error:
        # 无法算出差异时（例如只 fetch 了合并引用的外部 PR）宁可跑全量测试，
        # 也不能把发布快速通道当成默认结论。
        print(f"::warning::cannot diff {base}..{head}: {error}")
        print("release-only: false (diff unavailable, running the full matrix)")
        return

    release_only, reason = classify(files, diff)
    print(f"changed files: {', '.join(files) if files else '<none>'}")
    print(f"release-only: {release_only} ({reason})")

    version = ""
    if release_only:
        version = verify_release_metadata(root, read(root / CHANGELOG_PATH))
        print(f"release metadata validated for v{version}")

    output_path = arguments.github_output or os.environ.get("GITHUB_OUTPUT", "")
    if release_only and output_path:
        write_github_output(
            Path(output_path),
            {"release_only": "true", "version": version},
        )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    check_parser = subparsers.add_parser(
        "check", help="classify a pull request and validate release-only contents"
    )
    check_parser.add_argument("--base", default="")
    check_parser.add_argument("--head", default="")
    check_parser.add_argument("--root", type=Path, default=Path.cwd())
    check_parser.add_argument("--github-output", default="")
    check_parser.set_defaults(handler=command_check)

    return parser


def main() -> int:
    parser = build_parser()
    arguments = parser.parse_args()
    try:
        arguments.handler(arguments)
    except ReleasePrError as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
