#!/usr/bin/env python3

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


DEPENDENCIES = {
    "gpui-component": "gpui_ce_components",
    "gpui-component-assets": "gpui_ce_components_assets",
    "gpui-base": "gpui_ce_components_base",
    "gpui-shell": "gpui_ce_components_shell",
    "gpui-component-shell": "gpui-component-shell",
}

SHELL_VERSION_FILES = (
    "docs/extension-resource-plugins/gpui-shell-extension-design.md",
    "crates/extension-runtime/src/extension/manifest/parser_tests.rs",
    "crates/extension-runtime/src/extension_runtime_contract_tests.rs",
)


def run(command: list[str], cwd: Path) -> str:
    result = subprocess.run(
        command,
        cwd=cwd,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    return result.stdout.strip()


def resolve_revision(component_dir: Path, revision: str) -> str:
    resolved = run(
        ["git", "rev-parse", "--verify", f"{revision}^{{commit}}"],
        component_dir,
    )
    if not re.fullmatch(r"[0-9a-f]{40}", resolved):
        raise ValueError(f"invalid resolved revision: {resolved}")
    return resolved


def package_versions(component_dir: Path, offline: bool) -> dict[str, str]:
    command = [
        "cargo",
        "metadata",
        "--manifest-path",
        str(component_dir / "Cargo.toml"),
        "--format-version",
        "1",
        "--no-deps",
    ]
    if offline:
        command.append("--offline")
    metadata = json.loads(run(command, component_dir))
    versions = {package["name"]: package["version"] for package in metadata["packages"]}
    missing = sorted(set(DEPENDENCIES.values()) - versions.keys())
    if missing:
        raise ValueError(f"component metadata is missing packages: {', '.join(missing)}")
    return {alias: versions[package] for alias, package in DEPENDENCIES.items()}


def update_manifest(text: str, revision: str, versions: dict[str, str]) -> str:
    lines = text.splitlines(keepends=True)
    updated = set()
    for index, line in enumerate(lines):
        for alias, version in versions.items():
            if not line.startswith(f"{alias} = ") or "gpui-component.git" not in line:
                continue
            next_line, rev_count = re.subn(
                r'\brev\s*=\s*"[^"]+"', f'rev = "{revision}"', line, count=1
            )
            next_line, version_count = re.subn(
                r'\bversion\s*=\s*"[^"]+"',
                f'version = "{version}"',
                next_line,
                count=1,
            )
            if rev_count != 1 or version_count != 1:
                raise ValueError(f"cannot update dependency line for {alias}")
            lines[index] = next_line
            updated.add(alias)
            break
    missing = sorted(set(versions) - updated)
    if missing:
        raise ValueError(f"Navop manifest is missing dependencies: {', '.join(missing)}")
    return "".join(lines)


def replace_once(text: str, pattern: str, replacement: str, label: str) -> str:
    updated, count = re.subn(pattern, replacement, text, count=1, flags=re.MULTILINE)
    if count != 1:
        raise ValueError(f"expected one {label}, found {count}")
    return updated


def update_shell_version(text: str, shell_version: str, label: str) -> str:
    text, json_count = re.subn(
        r'("gpui_shell"\s*:\s*")[^"]+("\s*)',
        rf"\g<1>{shell_version}\g<2>",
        text,
    )
    text, rust_count = re.subn(
        r'(gpui_shell:\s*")[^"]+("\.to_string\(\))',
        rf"\g<1>{shell_version}\g<2>",
        text,
    )
    if json_count + rust_count == 0:
        raise ValueError(f"cannot find gpui-shell version in {label}")
    return text


def desired_files(
    navop_root: Path, component_dir: Path, revision: str, offline: bool
) -> tuple[dict[Path, str], str]:
    versions = package_versions(component_dir, offline)
    shell_version = versions["gpui-shell"]
    changes: dict[Path, str] = {}

    manifest = navop_root / "Cargo.toml"
    changes[manifest] = update_manifest(manifest.read_text(), revision, versions)

    external_doc = navop_root / "docs/gpui-component-external.md"
    changes[external_doc] = replace_once(
        external_doc.read_text(),
        r"^rev [0-9a-f]{7,40}$",
        f"rev {revision}",
        "external component revision",
    )

    design_doc = navop_root / "docs/extension-resource-plugins/gpui-shell-extension-design.md"
    design = replace_once(
        design_doc.read_text(),
        r"固定 fork `[0-9a-f]{7,40}`",
        f"固定 fork `{revision}`",
        "design component revision",
    )
    changes[design_doc] = design

    validation = navop_root / "crates/extension-runtime/src/extension/manifest/shell_validation.rs"
    changes[validation] = replace_once(
        validation.read_text(),
        r'const GPUI_SHELL_VERSION: &str = "[^"]+";',
        f'const GPUI_SHELL_VERSION: &str = "{shell_version}";',
        "gpui-shell host version",
    )

    for relative in SHELL_VERSION_FILES:
        path = navop_root / relative
        text = changes.get(path, path.read_text())
        changes[path] = update_shell_version(text, shell_version, relative)

    return changes, shell_version


def write_atomic(path: Path, text: str) -> None:
    temporary = path.with_name(f".{path.name}.gpui-component-update")
    temporary.write_text(text)
    temporary.replace(path)


def update_lock(navop_root: Path, offline: bool) -> None:
    command = ["cargo", "update"]
    if offline:
        command.append("--offline")
    for package in DEPENDENCIES.values():
        command.extend(["-p", package])
    subprocess.run(command, cwd=navop_root, check=True)


def lock_matches(navop_root: Path, revision: str) -> bool:
    lock = (navop_root / "Cargo.lock").read_text()
    pattern = re.compile(
        rf'git\+https://github\.com/feigeCode/gpui-component\.git\?rev={revision}#{revision}'
    )
    return len(pattern.findall(lock)) >= len(DEPENDENCIES)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Update Navop to one gpui-component revision and package version set."
    )
    parser.add_argument("--component-dir", required=True, type=Path)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--navop-root", type=Path)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--skip-lock", action="store_true")
    args = parser.parse_args()

    script_dir = Path(__file__).resolve().parent
    navop_root = (args.navop_root or script_dir.parent).resolve()
    component_dir = args.component_dir.resolve()
    revision = resolve_revision(component_dir, args.revision)
    component_head = resolve_revision(component_dir, "HEAD")
    if revision != component_head:
        raise ValueError(
            f"component checkout HEAD is {component_head}, not requested revision {revision}"
        )
    changes, shell_version = desired_files(
        navop_root, component_dir, revision, args.offline
    )
    changed = [path for path, text in changes.items() if path.read_text() != text]

    if args.check:
        if changed:
            for path in changed:
                print(f"outdated: {path.relative_to(navop_root)}")
            return 1
        if not args.skip_lock and not lock_matches(navop_root, revision):
            print("outdated: Cargo.lock")
            return 1
        print(f"gpui-component revision and versions are current: {revision}")
        return 0

    originals = {path: path.read_bytes() for path in changed}
    lock_path = navop_root / "Cargo.lock"
    if not args.skip_lock:
        originals[lock_path] = lock_path.read_bytes()
    try:
        for path in changed:
            write_atomic(path, changes[path])
        if not args.skip_lock:
            update_lock(navop_root, args.offline)
    except Exception:
        for path, content in originals.items():
            path.write_bytes(content)
        raise

    print(f"updated gpui-component revision: {revision}")
    print(f"updated gpui-shell compatibility version: {shell_version}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)
