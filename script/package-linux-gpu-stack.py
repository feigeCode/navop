#!/usr/bin/env python3

"""Package the standalone Linux GPU and desktop dependency stack for Navop.

The archive is assembled from unpacked distribution RPMs rather than from the
running system. That keeps packaging architecture independent: a single
x86_64 CI runner can produce both the x86_64 and the aarch64 archive, because
nothing in here executes the collected libraries, it only reads their ELF
headers.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import fnmatch
import glob
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
from typing import Iterable, NoReturn


# Navop reaches EGL through dlopen, so nothing in navop's own DT_NEEDED closes
# over the graphics stack. These entry points are collected explicitly; the
# recursive expansion then pulls in the rest of Mesa (the swrast driver NEEDs
# libLLVM, for example).
GPU_STACK_ENTRY_LIBRARIES = (
    "libEGL.so.1",
    "libEGL_mesa.so.0",
)


# Mesa loads its DRI driver by file name from a compile-time search directory,
# which is why the driver is resolved by directory scan instead of by SONAME.
GPU_STACK_DRI_DRIVERS = (
    "swrast_dri.so",
    "kms_swrast_dri.so",
)


# GLVND reads this to discover the Mesa vendor library installed alongside it.
GPU_STACK_EGL_VENDOR_CONFIGURATION = "/usr/share/glvnd/egl_vendor.d/50_mesa.json"


# Navop runs on the host dynamic loader, so the C runtime must come from the
# host. Bundling these would shadow the system glibc with a foreign copy, which
# is precisely what this archive must never do.
HOST_PROVIDED_LIBRARIES = (
    "ld-linux-aarch64.so.1",
    "ld-linux-x86-64.so.2",
    "libc.so.6",
    "libdl.so.2",
    "libm.so.6",
    "libpthread.so.0",
    "libresolv.so.2",
    "librt.so.1",
)

HOST_PROVIDED_PATTERNS = (
    "libnss_*.so.2",
    "linux-vdso.so.1",
)


# GTK, WebKitGTK and the GLib family integrate with the running desktop: themes,
# the accessibility bus, dconf, input methods, font configuration. Shipping one
# distribution's copy of that stack onto another distribution's desktop is not
# supportable, and it would dwarf the renderer this archive exists to deliver.
# Navop's release binary links the embedded webview, so these names are reached
# on every walk; they are recorded as host requirements and belong in the
# package's declared dependencies instead.
HOST_DESKTOP_PATTERNS = (
    "libwebkit2gtk-4*.so.*",
    "libjavascriptcoregtk-4*.so.*",
    "libgtk-3.so.*",
    "libgtk-4.so.*",
    "libgdk-3.so.*",
    "libgdk-4.so.*",
    "libsoup-2.4.so.*",
    "libsoup-3.0.so.*",
    "libglib-2.0.so.*",
    "libgobject-2.0.so.*",
    "libgio-2.0.so.*",
    "libgmodule-2.0.so.*",
    "libgthread-2.0.so.*",
    "libpango-1.0.so.*",
    "libpangocairo-1.0.so.*",
    "libcairo.so.*",
    "libcairo-gobject.so.*",
    "libharfbuzz.so.*",
    "libatk-1.0.so.*",
    "libatk-bridge-2.0.so.*",
    "libgdk_pixbuf-2.0.so.*",
    "libepoxy.so.*",
)


@dataclass(frozen=True)
class TargetConfig:
    machine: str
    architecture_label: str
    libdir: str
    dri_dir: str
    library_directories: tuple[str, ...]
    dri_driver_directories: tuple[str, ...]


# The archive is built from a RHEL 8 generation distribution, where both
# architectures use /usr/lib64. That path is what Mesa's compile-time DRI
# directory points at, so the payload keeps it verbatim; the installer
# registers the directory with the loader on hosts that do not search it.
TARGET_CONFIGS = {
    "aarch64-unknown-linux-gnu": TargetConfig(
        machine="AArch64",
        architecture_label="aarch64",
        libdir="/usr/lib64",
        dri_dir="/usr/lib64/dri",
        library_directories=("/usr/lib64", "/lib64", "/usr/lib", "/usr/local/lib"),
        dri_driver_directories=(
            "/usr/lib64/dri",
            "/usr/lib/dri",
            "/usr/lib/aarch64-linux-gnu/dri",
        ),
    ),
    "x86_64-unknown-linux-gnu": TargetConfig(
        machine="Advanced Micro Devices X86-64",
        architecture_label="x86_64",
        libdir="/usr/lib64",
        dri_dir="/usr/lib64/dri",
        library_directories=("/usr/lib64", "/lib64", "/usr/lib", "/usr/local/lib"),
        dri_driver_directories=(
            "/usr/lib64/dri",
            "/usr/lib/dri",
            "/usr/lib/x86_64-linux-gnu/dri",
        ),
    ),
}


GL_GROUP = "gl"
CLIENT_GROUP = "client"


@dataclass
class CollectedLibrary:
    soname: str
    source: Path
    group: str


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Collect the Mesa software renderer and the desktop client "
            "libraries Navop needs on a host without a usable GPU driver, and "
            "lay them out as an installable dependency archive."
        )
    )
    parser.add_argument(
        "--binary",
        required=True,
        type=Path,
        help=(
            "release Navop binary; its DT_NEEDED closure determines the "
            "desktop client libraries that must be shipped"
        ),
    )
    parser.add_argument(
        "--library-root",
        required=True,
        type=Path,
        help="root of the unpacked distribution file tree to collect from",
    )
    parser.add_argument(
        "--rpm-directory",
        required=True,
        type=Path,
        help="directory holding the downloaded RPMs, used for package metadata",
    )
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--installer-source",
        type=Path,
        default=Path("script/linux-gpu-stack-install.sh"),
        help="installer script copied to the archive root as install.sh",
    )
    parser.add_argument(
        "--target",
        default="aarch64-unknown-linux-gnu",
        choices=tuple(TARGET_CONFIGS),
    )
    parser.add_argument(
        "--glibc-baseline",
        default="2.28",
        help=(
            "maximum GLIBC symbol version allowed in any bundled library; the "
            "archive must stay loadable on the oldest host Navop itself runs on"
        ),
    )
    parser.add_argument(
        "--source-distribution",
        default="",
        help="human readable build host description recorded in the manifest",
    )
    return parser.parse_args()


def fail(message: str) -> NoReturn:
    raise SystemExit(f"Error: {message}")


def run(command: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        check=check,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


def require_command(command: str) -> None:
    if shutil.which(command) is None:
        fail(f"required command is not installed: {command}")


def readelf(path: Path, *arguments: str) -> str:
    result = run(["readelf", *arguments, str(path)], check=False)
    if result.returncode != 0:
        fail(f"readelf failed for {path}: {result.stderr.strip()}")
    return result.stdout


def elf_machine(path: Path) -> str:
    header = readelf(path, "-hW")
    match = re.search(r"^\s*Machine:\s*(.+?)\s*$", header, re.MULTILINE)
    if match is None:
        fail(f"cannot determine ELF machine for {path}")
    return match.group(1)


def version_tuple(version: str) -> tuple[int, ...]:
    return tuple(int(component) for component in version.split("."))


def highest_glibc_version(path: Path) -> str | None:
    versions = re.findall(
        r"\bGLIBC_(\d+(?:\.\d+)+)\b",
        readelf(path, "--version-info", "-W"),
    )
    if not versions:
        return None
    return max(versions, key=version_tuple)


def verify_glibc_baseline(path: Path, maximum: str) -> str | None:
    highest = highest_glibc_version(path)
    if highest is None:
        return None
    if version_tuple(highest) > version_tuple(maximum):
        fail(
            f"{path} requires GLIBC_{highest}, above the supported "
            f"GLIBC_{maximum} archive baseline; the dependency stack must come "
            "from a distribution at or below that baseline"
        )
    return highest


def dynamic_metadata(path: Path) -> list[str]:
    return re.findall(
        r"\(NEEDED\).*?Shared library:\s*\[([^\]]+)\]",
        readelf(path, "-dW"),
    )


def is_host_provided(soname: str) -> bool:
    if soname in HOST_PROVIDED_LIBRARIES:
        return True
    return any(fnmatch.fnmatch(soname, pattern) for pattern in HOST_PROVIDED_PATTERNS)


def is_host_desktop_library(soname: str) -> bool:
    return any(
        fnmatch.fnmatch(soname, pattern) for pattern in HOST_DESKTOP_PATTERNS
    )


def inside(root: Path, path: Path) -> bool:
    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


class LibraryIndex:
    """Indexes every shared object below an unpacked distribution tree.

    A fixed list of search directories is not sufficient. RHEL 8 keeps its
    compat LLVM runtime in /usr/lib64/llvm17/lib, a directory no dynamic loader
    configuration searches, yet Mesa's single gallium driver links against the
    libLLVM-17.so it holds. Indexing the tree by file name keeps the packager
    independent of wherever a distribution decides to park a library, while the
    configured directories still decide which copy wins when a name exists in
    more than one place.
    """

    def __init__(self, root: Path) -> None:
        self.root = root
        self._by_name: dict[str, list[Path]] = {}
        for path in root.rglob("*"):
            name = path.name
            if ".so" not in name or not path.is_file():
                continue
            resolved = path.resolve()
            if not inside(root, resolved) or not resolved.is_file():
                continue
            # Keep the path as packaged rather than its target: RPM file lists
            # name the symlink (/usr/lib64/libgcc_s.so.1), so ownership lookup
            # and the archive layout both have to use the same spelling.
            self._by_name.setdefault(name, []).append(path)

    def candidates(self, name: str, directories: Iterable[str]) -> list[Path]:
        found = self._by_name.get(name)
        if not found:
            return []
        wanted = list(directories)
        preferred: list[Path] = []
        remaining: list[Path] = []
        for path in found:
            parent = "/" + path.relative_to(self.root).parent.as_posix()
            if parent in wanted:
                preferred.append(path)
            else:
                remaining.append(path)
        preferred.sort(
            key=lambda path: wanted.index(
                "/" + path.relative_to(self.root).parent.as_posix()
            )
        )
        remaining.sort(
            key=lambda path: (len(path.relative_to(self.root).parts), str(path))
        )
        return preferred + remaining


def resolve_library(
    soname: str,
    *,
    index: LibraryIndex,
    directories: Iterable[str],
    machine: str,
) -> Path | None:
    for candidate in index.candidates(soname, directories):
        if elf_machine(candidate) == machine:
            return candidate
    return None


def resolve_dri_driver(
    name: str,
    *,
    index: LibraryIndex,
    directories: Iterable[str],
) -> Path | None:
    # Driver directories hold one real module plus per-driver symlinks, so the
    # index resolves each name through to the file it points at.
    for candidate in index.candidates(name, directories):
        return candidate
    return None


# RHEL 8 is a merged-/usr distribution: /lib64 is a symlink to /usr/lib64, and
# the RPM file lists still name the pre-merge path. libgcc is the visible case,
# declaring /lib64/libgcc_s.so.1 while the extracted tree stores
# /usr/lib64/libgcc_s.so.1. Ownership lookup has to accept both spellings.
USR_MERGE_PREFIXES = (
    ("/lib64/", "/usr/lib64/"),
    ("/lib/", "/usr/lib/"),
)


def path_spellings(path: str) -> list[str]:
    variants = {path}
    for merged, canonical in USR_MERGE_PREFIXES:
        if path.startswith(merged):
            variants.add(canonical + path[len(merged):])
        elif path.startswith(canonical):
            variants.add(merged + path[len(canonical):])
    return sorted(variants)


class RpmIndex:
    """Maps unpacked file paths back to the RPM that owns them."""

    def __init__(self, directory: Path) -> None:
        self.owners: dict[str, str] = {}
        self.archives: dict[str, Path] = {}
        for rpm in sorted(directory.glob("*.rpm")):
            name = self.field(rpm, "%{NAME}")
            if not name:
                continue
            self.archives[name] = rpm
            for path in self.paths(rpm):
                for spelling in path_spellings(path):
                    self.owners.setdefault(spelling, name)

    @staticmethod
    def field(rpm: Path, query_format: str) -> str | None:
        result = run(["rpm", "-qp", "--qf", query_format, str(rpm)], check=False)
        if result.returncode != 0:
            return None
        value = result.stdout.strip()
        return value or None

    @staticmethod
    def paths(rpm: Path) -> list[str]:
        result = run(["rpm", "-qlp", str(rpm)], check=False)
        if result.returncode != 0:
            return []
        return [line.strip() for line in result.stdout.splitlines() if line.strip()]

    def package_of(self, library_root: Path, path: Path) -> str | None:
        relative = path.relative_to(library_root)
        for spelling in path_spellings(f"/{relative.as_posix()}"):
            owner = self.owners.get(spelling)
            if owner is not None:
                return owner
        return None


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def collect_closure(
    roots: list[tuple[Path, str]],
    *,
    library_root: Path,
    index: LibraryIndex,
    target: TargetConfig,
    host_requirements: dict[str, str] | None = None,
) -> dict[str, CollectedLibrary]:
    """Expand DT_NEEDED from every root, remembering which group reached it.

    Client roots are processed first so a library needed by both Navop and
    Mesa is classified as a client library. That matters because client
    libraries are always reconciled against the host, while the graphics group
    is skipped wholesale when the host already renders.
    """
    collected: dict[str, CollectedLibrary] = {}
    scanned: set[Path] = set()
    queue: list[tuple[Path, str]] = list(roots)

    while queue:
        consumer, group = queue.pop(0)
        resolved_consumer = consumer.resolve()
        if resolved_consumer in scanned:
            continue
        scanned.add(resolved_consumer)

        if elf_machine(resolved_consumer) != target.machine:
            fail(f"architecture mismatch in dependency closure: {resolved_consumer}")

        for soname in dynamic_metadata(resolved_consumer):
            if is_host_provided(soname) or soname in collected:
                continue
            if is_host_desktop_library(soname):
                if host_requirements is not None:
                    host_requirements.setdefault(soname, str(resolved_consumer))
                continue
            source = resolve_library(
                soname,
                index=index,
                directories=target.library_directories,
                machine=target.machine,
            )
            if source is None:
                fail(
                    f"missing required shared library {soname} for "
                    f"{resolved_consumer}; download the RPM that provides it"
                )
            collected[soname] = CollectedLibrary(
                soname=soname,
                source=source,
                group=group,
            )
            queue.append((source, group))

    return collected


# Distributions disagree about where a package keeps its license text. RHEL 8
# uses /usr/share/licenses/<name> for some packages and /usr/share/doc/<name>
# for others, and either may carry a version suffix.
LICENSE_DIRECTORY_CANDIDATES = ("usr/share/licenses", "usr/share/doc")


def license_files_for(library_root: Path, package: str) -> list[Path]:
    found: dict[str, Path] = {}
    for relative in LICENSE_DIRECTORY_CANDIDATES:
        base = library_root / relative
        if not base.is_dir():
            continue
        for directory in sorted(base.glob(f"{glob.escape(package)}*")):
            if not directory.is_dir():
                continue
            for path in sorted(directory.rglob("*")):
                if path.is_file() and path.name not in found:
                    found[path.name] = path
    return [found[name] for name in sorted(found)]


def package_records(
    packaged: dict[str, Path],
    *,
    library_root: Path,
    index: RpmIndex,
    license_directory: Path,
) -> list[dict[str, object]]:
    by_package: dict[str, set[str]] = {}
    for relative, source in packaged.items():
        owner = index.package_of(library_root, source)
        if owner is None:
            fail(
                "cannot publish a bundled library without RPM ownership "
                f"metadata: {source}"
            )
        by_package.setdefault(owner, set()).add(relative)

    license_directory.mkdir(parents=True, exist_ok=True)
    records: list[dict[str, object]] = []
    for package in sorted(by_package):
        declared = index.archives.get(package)
        license_files = license_files_for(library_root, package)
        if license_files:
            destination = license_directory / package
            destination.mkdir(parents=True, exist_ok=True)
            for source in license_files:
                shutil.copy2(source, destination / source.name)
        else:
            print(
                f"warning: {package} ships no license files under "
                "/usr/share/licenses or /usr/share/doc; recording the "
                "declared license only",
                file=sys.stderr,
            )

        records.append(
            {
                "package": package,
                "version": (
                    RpmIndex.field(declared, "%{VERSION}-%{RELEASE}")
                    if declared
                    else "unknown"
                ),
                "license": (
                    RpmIndex.field(declared, "%{LICENSE}") if declared else "unknown"
                ),
                "license_files": sorted(p.name for p in license_files),
                "files": sorted(by_package[package]),
            }
        )
    return records


def manifest_files(output: Path, excluded: set[Path]) -> list[dict[str, object]]:
    records: list[dict[str, object]] = []
    for path in sorted(output.rglob("*")):
        if not path.is_file() or path in excluded:
            continue
        records.append(
            {
                "path": path.relative_to(output).as_posix(),
                "size": path.stat().st_size,
                "sha256": sha256(path),
            }
        )
    return records


def main() -> None:
    args = parse_args()
    target = TARGET_CONFIGS[args.target]
    for command in ("readelf", "rpm"):
        require_command(command)

    library_root = args.library_root.resolve()
    if not library_root.is_dir():
        fail(f"library root does not exist: {library_root}")
    rpm_directory = args.rpm_directory.resolve()
    if not rpm_directory.is_dir():
        fail(f"RPM directory does not exist: {rpm_directory}")

    binary = args.binary.resolve()
    if not binary.is_file():
        fail(f"release binary does not exist: {binary}")
    if elf_machine(binary) != target.machine:
        fail(
            f"dependency archive for {args.target} expects {target.machine}, "
            f"got {elf_machine(binary)}"
        )
    verify_glibc_baseline(binary, args.glibc_baseline)

    output = args.output.resolve()
    repository_root = Path(__file__).resolve().parent.parent
    if output in (Path("/"), repository_root):
        fail(f"refusing to replace unsafe output directory: {output}")
    installer_source = args.installer_source
    if not installer_source.is_absolute():
        installer_source = repository_root / installer_source
    if not installer_source.is_file():
        fail(f"installer script does not exist: {installer_source}")

    index = RpmIndex(rpm_directory)
    library_index = LibraryIndex(library_root)
    host_requirements: dict[str, str] = {}

    # Client libraries first: they are always reconciled against the host.
    libraries = collect_closure(
        [(binary, CLIENT_GROUP)],
        library_root=library_root,
        index=library_index,
        target=target,
        host_requirements=host_requirements,
    )

    graphics_roots: list[tuple[Path, str]] = []
    graphics_entries: dict[str, Path] = {}
    for soname in GPU_STACK_ENTRY_LIBRARIES:
        if soname in libraries:
            continue
        source = resolve_library(
            soname,
            index=library_index,
            directories=target.library_directories,
            machine=target.machine,
        )
        if source is None:
            fail(
                f"missing {soname} required by the graphics stack; download "
                "the RPM that provides the Mesa EGL runtime"
            )
        graphics_entries[soname] = source
        graphics_roots.append((source, GL_GROUP))

    drivers: dict[str, Path] = {}
    for name in GPU_STACK_DRI_DRIVERS:
        source = resolve_dri_driver(
            name,
            index=library_index,
            directories=target.dri_driver_directories,
        )
        if source is None:
            if name == GPU_STACK_DRI_DRIVERS[0]:
                fail(
                    f"missing {name} required by the software renderer; "
                    "download the RPM that provides the Mesa DRI drivers"
                )
            continue
        drivers[name] = source
        graphics_roots.append((source, GL_GROUP))

    for soname, entry in collect_closure(
        graphics_roots,
        library_root=library_root,
        index=library_index,
        target=target,
        host_requirements=host_requirements,
    ).items():
        libraries.setdefault(soname, entry)

    # The entry points are payload in their own right. Navop reaches EGL through
    # dlopen, so nothing in the binary's own DT_NEEDED closure pulls them in;
    # the walk above therefore only ever visits them as roots and would never
    # ship them.
    for soname, source in graphics_entries.items():
        libraries.setdefault(
            soname,
            CollectedLibrary(soname=soname, source=source, group=GL_GROUP),
        )

    vendor_source = library_root / GPU_STACK_EGL_VENDOR_CONFIGURATION.lstrip("/")
    if not vendor_source.is_file():
        fail(
            "missing EGL vendor configuration for the software renderer: "
            f"{GPU_STACK_EGL_VENDOR_CONFIGURATION}"
        )

    payload = output / "payload"
    packaged: dict[str, Path] = {}
    manifest_rows: list[tuple[str, str, str, str]] = []

    stored_by_digest: dict[str, str] = {}

    def stage(relative: str, source: Path, kind: str, group: str) -> None:
        destination = payload / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        digest = sha256(source)
        owner = stored_by_digest.get(digest)
        if owner is not None:
            # Distributions hard-link one real module under several names: RHEL
            # 8 exposes six DRI drivers that are all the same 19 MiB inode.
            # Filing the duplicate as a relative symlink keeps the archive from
            # carrying that payload twice, and the installer dereferences it.
            destination.symlink_to(os.path.relpath(payload / owner, destination.parent))
        else:
            shutil.copy2(source, destination)
            destination.chmod(0o644)
            stored_by_digest[digest] = relative
        packaged[relative] = source
        manifest_rows.append((kind, relative, group, digest))

    for soname, entry in sorted(libraries.items()):
        stage(f"{target.libdir.lstrip('/')}/{soname}", entry.source, "library", entry.group)
    for name, source in sorted(drivers.items()):
        stage(f"{target.dri_dir.lstrip('/')}/{name}", source, "driver", GL_GROUP)
    vendor_relative = GPU_STACK_EGL_VENDOR_CONFIGURATION.lstrip("/")
    stage(vendor_relative, vendor_source, "vendor", GL_GROUP)

    # Every bundled shared object must stay inside the glibc baseline, because
    # the host loader resolves them directly now.
    baselines: dict[str, str] = {}
    for relative, source in sorted(packaged.items()):
        if relative == vendor_relative:
            continue
        highest = verify_glibc_baseline(source, args.glibc_baseline)
        if highest is not None:
            baselines[relative] = highest

    shutil.copy2(installer_source, output / "install.sh")
    (output / "install.sh").chmod(0o755)

    distribution = args.source_distribution or "RHEL 8 generation distribution"
    # install.sh sources this file, so values are shell-quoted: the source
    # distribution string carries spaces and parentheses.
    (output / "install.env").write_text(
        "".join(
            f"{key}={shlex.quote(value)}\n"
            for key, value in (
                ("NAVOP_GPU_STACK_ARCH", target.architecture_label),
                ("NAVOP_GPU_STACK_TARGET", args.target),
                ("NAVOP_GPU_STACK_LIBDIR", target.libdir),
                ("NAVOP_GPU_STACK_DRI_DIR", target.dri_dir),
                (
                    "NAVOP_GPU_STACK_EGL_VENDOR_DIR",
                    str(Path(GPU_STACK_EGL_VENDOR_CONFIGURATION).parent),
                ),
                ("NAVOP_GPU_STACK_GLIBC_BASELINE", args.glibc_baseline),
                ("NAVOP_GPU_STACK_SOURCE_DISTRIBUTION", distribution),
            )
        ),
        encoding="utf-8",
    )

    (output / "manifest.tsv").write_text(
        "".join(
            f"{kind}\t{relative}\t{group}\t{digest}\n"
            for kind, relative, group, digest in manifest_rows
        ),
        encoding="utf-8",
    )

    records = package_records(
        packaged,
        library_root=library_root,
        index=index,
        license_directory=output / "licenses",
    )
    (output / "runtime-packages.txt").write_text(
        "".join(
            f"{record['package']}\t{record['version']}\t{','.join(record['files'])}\n"
            for record in records
        ),
        encoding="utf-8",
    )

    manifest_path = output / "manifest.json"
    graphics_libraries = sorted(
        soname for soname, entry in libraries.items() if entry.group == GL_GROUP
    )
    client_libraries = sorted(
        soname for soname, entry in libraries.items() if entry.group == CLIENT_GROUP
    )
    manifest = {
        "schema_version": 1,
        "product": "navop",
        "component": "linux-gpu-stack",
        "target": args.target,
        "architecture": target.architecture_label,
        "source_distribution": distribution,
        "binary_glibc_baseline": args.glibc_baseline,
        "libdir": target.libdir,
        "dri_dir": target.dri_dir,
        "egl_vendor_dir": str(Path(GPU_STACK_EGL_VENDOR_CONFIGURATION).parent),
        "graphics_libraries": graphics_libraries,
        "client_libraries": client_libraries,
        "host_required_libraries": sorted(host_requirements),
        "dri_drivers": sorted(drivers),
        "egl_vendor_configurations": [Path(GPU_STACK_EGL_VENDOR_CONFIGURATION).name],
        "glibc_requirements": baselines,
        "packages": records,
        "host_interfaces": [
            "Linux kernel and procfs",
            "X11 or Wayland display sockets",
            "GPU devices and host vendor drivers",
            "system fonts and font configuration",
            "the distribution's GTK and WebKitGTK stack, which the release "
            "binary links for the embedded webview",
        ],
        "files": manifest_files(output, {manifest_path}),
    }
    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

    payload_size = sum(
        path.stat().st_size for path in payload.rglob("*") if path.is_file()
    )
    print(
        f"Packaged {len(libraries)} libraries ({len(graphics_libraries)} graphics, "
        f"{len(client_libraries)} client) and {len(drivers)} DRI drivers into {output}"
    )
    if host_requirements:
        print(
            f"Left to the host ({len(host_requirements)} desktop libraries): "
            + ", ".join(sorted(host_requirements))
        )
    print(f"Uncompressed payload: {payload_size / (1024 * 1024):.1f} MiB")
    print(f"Built on: {distribution} (GLIBC_{args.glibc_baseline} baseline)")


if __name__ == "__main__":
    main()
