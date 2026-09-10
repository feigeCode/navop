#!/usr/bin/env python3
"""Publish a local gpui-pre snapshot to a git fork repository.

`gpui-pre-*` is the crate name under which gpui-component republishes Zed's
gpui crates to crates.io. The publisher (`gpui-component/script/bump-gpui.ts`)
is the same pipeline Navop's `patch-local-gpui-pre.py` already runs in
`--stage-only` mode. The two scripts share a single staging area so the
fork is just a "stage, then commit, then push" extension of the existing
path-patch workflow.

Why bother when a path patch already works locally? A path patch ties
Navop to a directory on the build host. CI, teammates, and any deploy
that runs `cargo` from scratch will not have that directory and the build
breaks. A git fork lives in a normal git remote, and once Navop's
dependencies point at it (`gpui-pre = { git = "...", tag = "..." }`),
every build host resolves it the same way it resolves any other dep.

Both modes of consumption are valid:

- **Path patch** (existing): `script/patch-local-gpui-pre.py`. Devs and CI
  runners that share a workspace layout. No network round-trip; fastest
  edit-build loop.
- **Git fork** (this script + `migrate-to-git-fork.py`): the long-lived
  self-maintenance mode. CI, teammates, and a future "switch to upstream"
  flip all use the same source.

Usage:
    script/publish-gpui-pre-fork.py [options]

Options:
    --zed PATH         Zed checkout to snapshot (default: the `zed` checkout
                       next to Navop)
    --component PATH   gpui-component checkout that owns the staging pipeline
                       (default: the `gpui-component` checkout next to Navop)
    --patch-dir PATH   Where the staging script dropped the snapshot
                       (default: `<gpui-component>/../.gpui-pre`)
    --fork-url URL     Git URL of the fork. Anything `git push` accepts:
                       `https://github.com/feigeCode/gpui-pre.git` (the
                       repo is public; pushes need a token), or SSH.
    --branch NAME      Branch to push the snapshot to (default: main)
    --tag NAME         Tag to apply. Default: `fork-<version>`, e.g.
                       `fork-0.3.99`. Bump this to publish a new snapshot.
    --version VERSION  The version embedded in the snapshot and the default
                       tag. Must match what the staging pipeline stamped.
    --message MSG      Commit message; the version is appended automatically.
    --no-stage         Reuse the snapshot already produced by
                       `patch-local-gpui-pre.py`. Avoids a second
                       `bump-gpui.ts` run when the Zed branch has not moved.
    --init-only        Initialize the git repo and commit locally, but do not
                       push. Useful for inspecting what would go upstream.
    --dry-run          Print the commands without running them. Combine with
                       `--init-only` for a full preview.
    -h, --help         Show this help

Workflow:

    # One-time, on a fresh fork:
    script/publish-gpui-pre-fork.py \\
        --fork-url https://github.com/feigeCode/gpui-pre.git \\
        --init-only            # commit only, inspect
    # (push manually, or drop --init-only once you trust the snapshot)

    # When the local Zed branch changes:
    script/patch-local-gpui-pre.py --no-stage       # refreshes the staging
    script/publish-gpui-pre-fork.py \\
        --fork-url https://github.com/feigeCode/gpui-pre.git \\
        --tag fork-0.3.100       # bumped version

    # In Navop, switch the dependency:
    script/migrate-to-git-fork.py                   # see its docstring

The shipped snapshot is a single commit on `--branch`, tagged `--tag`. All
25 gpui-pre-* crates share that commit and that tag. Cargo's git
dependency resolution keys on `package = "..."` for crate identity and on
the tag/ref for source identity, so a single tag is enough.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

DEFAULT_TAG_PREFIX = "fork"
DEFAULT_BRANCH = "main"
PUBLISH_PREFIX = "gpui-pre"
DEFAULT_VERSION = "0.3.99"


class Error(Exception):
    """A user-facing failure, printed without a traceback."""


def run(cmd: list[str], cwd: Path | None = None, dry_run: bool = False) -> None:
    print(f"$ {' '.join(cmd)}" + ("  (dry-run)" if dry_run else ""))
    if dry_run:
        return
    result = subprocess.run(cmd, cwd=cwd)
    if result.returncode != 0:
        raise Error(f"command failed with exit code {result.returncode}: {' '.join(cmd)}")


def run_capture(cmd: list[str], cwd: Path | None = None) -> str:
    result = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        raise Error(
            f"command failed with exit code {result.returncode}: {' '.join(cmd)}\n"
            f"{result.stderr}"
        )
    return result.stdout


def sibling(root: Path, name: str, marker: str, flag: str) -> Path:
    """Find a checkout next to the Navop root, at any of the nesting depths."""
    for depth in range(1, 5):
        candidate = root
        for _ in range(depth):
            candidate = candidate.parent
        candidate = candidate / name
        if (candidate / marker).exists():
            return candidate.resolve()
    raise Error(
        f"could not find a {name} checkout next to {root}; pass {flag} PATH"
    )


def find_bun() -> str:
    """Locate the Bun runtime that runs the staging pipeline."""
    found = shutil.which("bun")
    if found:
        return found
    fallback = Path.home() / "node_modules" / ".bin" / "bun"
    if fallback.is_file():
        return str(fallback)
    raise Error(
        "bun is not installed; install it with `npm install -g bun` (or "
        "`brew install oven-sh/bun/bun`) and re-run"
    )


def read_workspace_version(workspace: Path) -> str:
    """Pull `version` out of the staged root manifest.

    The pipeline stamps the version on every member, but a single source of
    truth keeps the publish script and the user from disagreeing about what
    was actually published. The version lives in `[workspace.dependencies]`
    as `version = "=0.3.99"`.
    """
    manifest = (workspace / "Cargo.toml").read_text()
    section = re.search(
        r"^\[workspace\.dependencies\]\s*$(.*?)(?=^\[|\Z)",
        manifest,
        re.MULTILINE | re.DOTALL,
    )
    if section is None:
        raise Error(f"{workspace / 'Cargo.toml'} has no [workspace.dependencies]")
    match = re.search(r'\bversion\s*=\s*"=([0-9][^"]*)"', section.group(1))
    if match is None:
        raise Error(f"could not find a pinned version in {workspace / 'Cargo.toml'}")
    return match.group(1)


def publish(
    workspace: Path,
    fork_url: str | None,
    branch: str,
    tag: str,
    message: str,
    version: str,
    init_only: bool,
    dry_run: bool,
    component: Path,
) -> Path:
    """Materialise a clean, publishable copy of the snapshot and push it.

    The copy lives next to the existing path-patch mirror (under
    `<component.parent>/.gpui-pre/publish`) so nothing leaks into the
    build output tree. The git history of that copy is the published
    history; rebuilds always start from the staged snapshot, so a
    previous `init` is preserved across runs of this script (bumping the
    tag creates a new commit on the same branch).
    """
    patch_dir = component.parent / ".gpui-pre"
    publish_root = patch_dir / "publish"
    # Accumulating history: wipe the copied files but keep `.git`, so each
    # publish is a new commit on the same branch and consecutive tags can be
    # diffed against each other.
    if publish_root.is_dir() and not dry_run:
        for child in publish_root.iterdir():
            if child.name == ".git":
                continue
            if child.is_dir():
                shutil.rmtree(child)
            else:
                child.unlink()
    elif not dry_run:
        publish_root.mkdir(parents=True)
    elif not publish_root.exists():
        # In dry-run we still want to point the user at a path, even though
        # the copy does not exist. The real copy is created when the run is
        # not a dry-run.
        publish_root.mkdir(parents=True, exist_ok=True)
    if not dry_run:
        shutil.copytree(
            workspace, publish_root, ignore=shutil.ignore_patterns("target", ".git"), dirs_exist_ok=True
        )

    git = shutil.which("git")
    if git is None:
        raise Error("git is not on PATH")

    is_fresh = not (publish_root / ".git").is_dir()
    if is_fresh:
        run([git, "init", "-b", branch, "--quiet"], cwd=publish_root, dry_run=dry_run)
        run(
            [git, "config", "user.email", "gpui-pre-fork@localhost"],
            cwd=publish_root,
            dry_run=dry_run,
        )
        run(
            [git, "config", "user.name", "gpui-pre fork publisher"],
            cwd=publish_root,
            dry_run=dry_run,
        )

    if fork_url and not init_only and not dry_run:
        has_origin = subprocess.run(
            [git, "remote", "get-url", "origin"], cwd=publish_root, capture_output=True
        ).returncode == 0
        if has_origin:
            existing = run_capture([git, "remote", "get-url", "origin"], cwd=publish_root).strip()
            if existing != fork_url:
                raise Error(
                    f"{publish_root} already has origin={existing!r}; pass a "
                    f"different branch, edit the remote, or delete .git to start over"
                )
        else:
            run([git, "remote", "add", "origin", fork_url], cwd=publish_root, dry_run=dry_run)

    run([git, "add", "-A"], cwd=publish_root, dry_run=dry_run)

    status = run_capture([git, "status", "--porcelain"], cwd=publish_root) if not dry_run else ""
    if status.strip() or is_fresh:
        full_message = f"{message}\n\nSnapshot of Zed's gpui crates, renamed to {PUBLISH_PREFIX}-*\n\nVersion: {version}"
        run([git, "commit", "-m", full_message], cwd=publish_root, dry_run=dry_run)

    existing_tags = (
        run_capture([git, "tag", "--list", tag], cwd=publish_root).strip()
        if not dry_run
        else ""
    )
    if existing_tags:
        # Re-publishing the same version is a no-op on the tag itself, but
        # if the snapshot changed we want the new commit on the branch to
        # carry the tag. Move the tag forward to the new HEAD.
        run([git, "tag", "-f", tag, "HEAD"], cwd=publish_root, dry_run=dry_run)
        action = "moved"
    else:
        run([git, "tag", tag, "HEAD"], cwd=publish_root, dry_run=dry_run)
        action = "created"

    if fork_url and not init_only and not dry_run:
        run(
            [git, "push", "--force-with-lease", "-u", "origin", f"HEAD:{branch}"],
            cwd=publish_root,
            dry_run=dry_run,
        )
        run([git, "push", "--force", "origin", f"refs/tags/{tag}"], cwd=publish_root, dry_run=dry_run)
    elif fork_url and not init_only and dry_run:
        print(f"# would push branch HEAD:{branch} and tag {tag} to {fork_url}")

    print(f"publish root: {publish_root}")
    print(f"branch: {branch} ({'updated' if not is_fresh else 'created'})")
    print(f"tag: {tag} ({action})")
    if init_only:
        print("init-only: nothing was pushed; re-run without --init-only to push")
    elif fork_url is None:
        print("no --fork-url given: nothing was pushed; pass --fork-url to push")
    else:
        print(f"pushed to {fork_url}")

    return publish_root


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--zed")
    parser.add_argument("--component")
    parser.add_argument("--patch-dir")
    parser.add_argument("--fork-url")
    parser.add_argument("--branch", default=DEFAULT_BRANCH)
    parser.add_argument("--tag")
    parser.add_argument("--version")
    parser.add_argument("--message", default="Snapshot gpui-pre from local Zed checkout")
    parser.add_argument("--no-stage", action="store_true")
    parser.add_argument("--init-only", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)

    root = Path(__file__).resolve().parent.parent
    component = (
        Path(args.component).expanduser().resolve()
        if args.component
        else sibling(root, "gpui-component", "script/bump-gpui.ts", "--component")
    )
    zed = (
        Path(args.zed).expanduser().resolve()
        if args.zed
        else sibling(root, "zed", "crates/gpui/Cargo.toml", "--zed")
    )

    # Reuse the staging pipeline directly; the path-patch script also runs
    # this pipeline but then goes on to write the path-patch block and run
    # `cargo update`, which we do not want here.
    if not args.no_stage:
        script = component / "script" / "bump-gpui.ts"
        run(
            [
                find_bun(),
                str(script),
                args.version or DEFAULT_VERSION,
                "--zed",
                str(zed),
                "--stage-only",
            ],
            cwd=component,
        )

    staged = component / "target" / "gpui-pre" / "workspace"
    if not (staged / "Cargo.toml").is_file():
        raise Error(f"no staged snapshot at {staged}; run without --no-stage")

    version = args.version or read_workspace_version(staged)
    tag = args.tag or f"{DEFAULT_TAG_PREFIX}-{version}"

    publish(
        workspace=staged,
        fork_url=args.fork_url,
        branch=args.branch,
        tag=tag,
        message=args.message,
        version=version,
        init_only=args.init_only,
        dry_run=args.dry_run,
        component=component,
    )

    print()
    print("Next, in Navop:")
    print(f"  script/migrate-to-git-fork.py --fork-url <url> --tag {tag}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except Error as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)
