#!/usr/bin/env python3

"""Build a relocatable Linux package around a private glibc runtime."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
from typing import Iterable, NoReturn


OPTIONAL_RUNTIME_LIBRARIES = (
    "libnss_dns.so.2",
    "libnss_files.so.2",
    "libresolv.so.2",
)

OPTIONAL_RUNTIME_PATTERNS = ("libnss_*.so.2",)

HOST_DRIVER_PATTERNS = (
    "libcuda.so*",
    "libnvidia-*.so*",
    "libamdhip64.so*",
    "libhsa-runtime64.so*",
    "libroc*.so*",
    "libigc.so*",
    "libze_*.so*",
    "libvulkan_*.so*",
)


USR_MERGE_PATH_ALIASES = (
    ("/bin", "/usr/bin"),
    ("/sbin", "/usr/sbin"),
    ("/lib", "/usr/lib"),
    ("/lib64", "/usr/lib64"),
)


COMMON_LIBRARY_DIRECTORIES = (
    "/lib64",
    "/lib",
    "/usr/lib64",
    "/usr/lib",
    "/usr/local/lib",
)


@dataclass(frozen=True)
class TargetConfig:
    machine: str
    loader: str
    platform_token: str
    lib_token: str
    library_directories: tuple[str, ...]


TARGET_CONFIGS = {
    "aarch64-unknown-linux-gnu": TargetConfig(
        machine="AArch64",
        loader="ld-linux-aarch64.so.1",
        platform_token="aarch64",
        lib_token="lib",
        library_directories=(
            "/lib/aarch64-linux-gnu",
            "/usr/lib/aarch64-linux-gnu",
            *COMMON_LIBRARY_DIRECTORIES,
        ),
    ),
    "x86_64-unknown-linux-gnu": TargetConfig(
        machine="Advanced Micro Devices X86-64",
        loader="ld-linux-x86-64.so.2",
        platform_token="x86_64",
        lib_token="lib64",
        library_directories=(
            "/lib/x86_64-linux-gnu",
            "/usr/lib/x86_64-linux-gnu",
            *COMMON_LIBRARY_DIRECTORIES,
        ),
    ),
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Bundle a Linux ELF with the runner's dynamic loader and "
            "recursive shared-library closure."
        )
    )
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--launcher-source", required=True, type=Path)
    parser.add_argument(
        "--target",
        default="aarch64-unknown-linux-gnu",
        choices=tuple(TARGET_CONFIGS),
        help="portable launcher target",
    )
    parser.add_argument(
        "--glibc-baseline",
        default="2.28",
        help=(
            "maximum GLIBC symbol version allowed in navop.real; the bundled "
            "private runtime itself may be newer"
        ),
    )
    return parser.parse_args()


def fail(message: str) -> NoReturn:
    raise SystemExit(f"Error: {message}")


def run(
    command: list[str],
    *,
    check: bool = True,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        check=check,
        env=env,
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


def elf_interpreter(path: Path) -> Path:
    program_headers = readelf(path, "-lW")
    matches = re.findall(
        r"Requesting program interpreter:\s*([^\]]+)",
        program_headers,
    )
    if len(matches) != 1:
        fail(f"expected one PT_INTERP entry in {path}, found {len(matches)}")
    return Path(matches[0])


def version_tuple(version: str) -> tuple[int, ...]:
    if re.fullmatch(r"\d+(?:\.\d+)+", version) is None:
        fail(f"invalid GLIBC baseline: {version}")
    return tuple(int(component) for component in version.split("."))


def verify_binary_glibc_baseline(path: Path, maximum: str) -> None:
    maximum_version = version_tuple(maximum)
    version_info = readelf(path, "--version-info", "-W")
    required_versions = {
        match
        for match in re.findall(r"\bGLIBC_(\d+(?:\.\d+)+)\b", version_info)
    }
    if not required_versions:
        fail(f"readelf did not report any GLIBC versions for {path}")

    highest = max(required_versions, key=version_tuple)
    if version_tuple(highest) > maximum_version:
        fail(
            f"{path} requires GLIBC_{highest}, above the supported "
            f"GLIBC_{maximum} binary baseline"
        )


def dynamic_metadata(
    path: Path,
    target: TargetConfig,
) -> tuple[list[str], list[Path]]:
    dynamic = readelf(path, "-dW")
    needed = re.findall(r"\(NEEDED\).*?Shared library:\s*\[([^\]]+)\]", dynamic)
    search_paths: list[Path] = []
    origin = path.resolve().parent

    for raw in re.findall(
        r"\((?:RPATH|RUNPATH)\).*?Library (?:rpath|runpath):\s*\[([^\]]*)\]",
        dynamic,
    ):
        for item in raw.split(":"):
            expanded = item
            for token, value in (
                ("ORIGIN", str(origin)),
                ("LIB", target.lib_token),
                ("PLATFORM", target.platform_token),
            ):
                expanded = expanded.replace(f"${{{token}}}", value).replace(
                    f"${token}",
                    value,
                )
            if expanded:
                search_paths.append(Path(expanded))

    return needed, search_paths


def ldconfig_cache() -> dict[str, list[Path]]:
    result = run(["ldconfig", "-p"], check=False)
    if result.returncode != 0:
        fail(f"ldconfig -p failed: {result.stderr.strip()}")

    cache: dict[str, list[Path]] = {}
    for line in result.stdout.splitlines():
        match = re.match(r"^\s*(\S+)\s+\([^)]+\)\s+=>\s+(\S+)\s*$", line)
        if match is None:
            continue
        cache.setdefault(match.group(1), []).append(Path(match.group(2)))
    return cache


def is_host_driver(soname: str) -> bool:
    return any(fnmatch.fnmatch(soname, pattern) for pattern in HOST_DRIVER_PATTERNS)


def optional_runtime_sonames(cache: dict[str, list[Path]]) -> list[str]:
    sonames = set(OPTIONAL_RUNTIME_LIBRARIES)
    sonames.update(
        soname
        for soname in cache
        if any(
            fnmatch.fnmatch(soname, pattern)
            for pattern in OPTIONAL_RUNTIME_PATTERNS
        )
    )
    return sorted(sonames)


def resolve_library(
    soname: str,
    *,
    consumer: Path,
    consumer_search_paths: Iterable[Path],
    cache: dict[str, list[Path]],
    machine: str,
    library_directories: Iterable[str],
) -> Path | None:
    candidates: list[Path] = []
    candidates.extend(path / soname for path in consumer_search_paths)
    candidates.extend(cache.get(soname, []))
    candidates.extend(Path(path) / soname for path in library_directories)

    seen: set[Path] = set()
    for candidate in candidates:
        try:
            resolved = candidate.resolve(strict=True)
        except (FileNotFoundError, OSError):
            continue
        if resolved in seen or not resolved.is_file():
            continue
        seen.add(resolved)
        try:
            candidate_machine = elf_machine(resolved)
        except SystemExit:
            continue
        if candidate_machine == machine:
            return resolved

    print(
        f"warning: unable to resolve {soname} required by {consumer}",
        file=sys.stderr,
    )
    return None


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def copy_runtime_file(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        if sha256(source) != sha256(destination):
            fail(
                "conflicting runtime libraries share the same bundled name: "
                f"{source} and {destination}"
            )
        return
    shutil.copy2(source, destination)
    destination.chmod(destination.stat().st_mode | 0o444)


def package_owner_query_paths(path: Path) -> tuple[Path, ...]:
    candidates: list[Path] = []

    def append(candidate: Path) -> None:
        if candidate not in candidates:
            candidates.append(candidate)

    append(path)
    try:
        append(path.resolve())
    except OSError:
        pass

    for candidate in tuple(candidates):
        candidate_text = str(candidate)
        for legacy_prefix, merged_prefix in USR_MERGE_PATH_ALIASES:
            for source_prefix, destination_prefix in (
                (legacy_prefix, merged_prefix),
                (merged_prefix, legacy_prefix),
            ):
                if candidate_text == source_prefix or candidate_text.startswith(
                    f"{source_prefix}/"
                ):
                    append(
                        Path(
                            f"{destination_prefix}"
                            f"{candidate_text[len(source_prefix):]}"
                        )
                    )

    return tuple(candidates)


def package_owner(path: Path) -> str | None:
    for candidate in package_owner_query_paths(path):
        result = run(["dpkg-query", "-S", str(candidate)], check=False)
        if result.returncode != 0:
            continue
        for line in result.stdout.splitlines():
            package, separator, _ = line.partition(": ")
            if separator and package:
                return package
    return None


def package_version(package: str) -> str:
    result = run(
        ["dpkg-query", "-W", "-f=${binary:Package}\t${Version}", package],
        check=False,
    )
    if result.returncode != 0:
        fail(f"cannot determine installed version for runtime package {package}")
    fields = result.stdout.strip().split("\t", 1)
    if len(fields) != 2 or not fields[1]:
        fail(f"invalid dpkg-query version output for runtime package {package}")
    return fields[1]


def copy_package_licenses(
    packaged_sources: dict[str, Path],
    license_directory: Path,
) -> list[dict[str, object]]:
    packages: dict[str, set[str]] = {}
    for bundled_name, source in packaged_sources.items():
        owner = package_owner(source)
        if owner is None:
            fail(
                "cannot publish a bundled runtime file without Debian package "
                f"ownership metadata: {source}"
            )
        packages.setdefault(owner, set()).add(bundled_name)

    license_directory.mkdir(parents=True, exist_ok=True)
    package_records: list[dict[str, object]] = []
    for package in sorted(packages):
        package_base = package.split(":", 1)[0]
        copyright_source = Path("/usr/share/doc") / package_base / "copyright"
        copyright_destination = license_directory / f"{package_base}.copyright"
        if copyright_source.is_file():
            shutil.copy2(copyright_source, copyright_destination)
        else:
            fail(
                "cannot publish bundled runtime package without its copyright "
                f"file: {package} ({copyright_source})"
            )

        package_records.append(
            {
                "package": package,
                "version": package_version(package),
                "files": sorted(packages[package]),
                "license": str(copyright_destination.name),
            }
        )
    return package_records


def compile_launcher(
    source: Path,
    destination: Path,
    expected_machine: str,
) -> str:
    destination.parent.mkdir(parents=True, exist_ok=True)
    run(
        [
            "musl-gcc",
            "-static",
            "-Os",
            "-s",
            "-Wl,--build-id=none",
            str(source),
            "-o",
            str(destination),
        ]
    )
    destination.chmod(0o755)
    if "Requesting program interpreter" in readelf(destination, "-lW"):
        fail(f"portable launcher is not static: {destination}")
    launcher_machine = elf_machine(destination)
    if launcher_machine != expected_machine:
        fail(
            "portable launcher architecture mismatch: "
            f"expected {expected_machine}, got {launcher_machine}"
        )
    return launcher_machine


def verify_private_runtime(loader: Path, library_directory: Path, binary: Path) -> None:
    environment = dict(os.environ)
    for name in ("LD_AUDIT", "LD_LIBRARY_PATH", "LD_PRELOAD", "LD_PROFILE"):
        environment.pop(name, None)

    verify = run([str(loader), "--verify", str(binary)], check=False, env=environment)
    if verify.returncode != 0:
        fail(
            "bundled loader rejected navop.real: "
            f"{verify.stderr.strip() or verify.stdout.strip()}"
        )

    listed = run(
        [
            str(loader),
            "--inhibit-cache",
            "--library-path",
            str(library_directory),
            "--list",
            str(binary),
        ],
        check=False,
        env=environment,
    )
    if listed.returncode != 0:
        fail(
            "bundled loader could not resolve navop.real dependencies: "
            f"{listed.stderr.strip() or listed.stdout.strip()}"
        )
    print(listed.stdout.rstrip())


def manifest_files(output: Path, manifest: Path) -> list[dict[str, object]]:
    records: list[dict[str, object]] = []
    for path in sorted(output.rglob("*")):
        if not path.is_file() or path == manifest:
            continue
        relative = path.relative_to(output).as_posix()
        records.append(
            {
                "path": relative,
                "size": path.stat().st_size,
                "sha256": sha256(path),
            }
        )
    return records


def main() -> None:
    args = parse_args()
    target = TARGET_CONFIGS[args.target]
    for command in ("dpkg-query", "ldconfig", "musl-gcc", "readelf"):
        require_command(command)

    binary = args.binary.resolve()
    launcher_source = args.launcher_source.resolve()
    output = args.output.resolve()
    repository_root = Path(__file__).resolve().parent.parent

    if not binary.is_file():
        fail(f"release binary does not exist: {binary}")
    if not launcher_source.is_file():
        fail(f"launcher source does not exist: {launcher_source}")
    if output == Path("/") or output == repository_root:
        fail(f"refusing to replace unsafe output directory: {output}")

    machine = elf_machine(binary)
    if machine != target.machine:
        fail(
            f"portable package for {args.target} expects {target.machine}, "
            f"got {machine}"
        )

    interpreter = elf_interpreter(binary)
    if interpreter.name != target.loader:
        fail(
            f"unexpected ELF interpreter for {args.target}: {interpreter}; "
            f"expected {target.loader}"
        )
    if not interpreter.is_file():
        fail(f"ELF interpreter does not exist on the build runner: {interpreter}")
    verify_binary_glibc_baseline(binary, args.glibc_baseline)

    if output.exists():
        shutil.rmtree(output)

    runtime_root = output / "usr/lib/navop"
    binary_destination = runtime_root / "bin/navop.real"
    library_directory = runtime_root / "lib"
    launcher_destination = output / "usr/bin/navop"
    documentation_directory = output / "usr/share/doc/navop"
    runtime_license_directory = documentation_directory / "runtime-licenses"

    binary_destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(binary, binary_destination)
    binary_destination.chmod(0o755)

    cache = ldconfig_cache()
    queue: list[Path] = [binary]
    scanned: set[Path] = set()
    runtime_sources: dict[str, Path] = {
        interpreter.name: interpreter.resolve(),
    }

    while queue:
        consumer = queue.pop(0).resolve()
        if consumer in scanned:
            continue
        scanned.add(consumer)
        if elf_machine(consumer) != machine:
            fail(f"runtime architecture mismatch: {consumer}")

        needed, search_paths = dynamic_metadata(consumer, target)
        for soname in needed:
            if is_host_driver(soname):
                fail(
                    f"{consumer} directly depends on host GPU driver {soname}; "
                    "portable packages may only use host drivers through runtime discovery"
                )
            source = resolve_library(
                soname,
                consumer=consumer,
                consumer_search_paths=search_paths,
                cache=cache,
                machine=machine,
                library_directories=target.library_directories,
            )
            if source is None:
                fail(f"missing required shared library {soname} for {consumer}")
            previous = runtime_sources.get(soname)
            if previous is not None and previous != source:
                if sha256(previous) != sha256(source):
                    fail(
                        f"{soname} resolves to conflicting files: {previous} and {source}"
                    )
            else:
                runtime_sources[soname] = source
            queue.append(source)

    for soname in optional_runtime_sonames(cache):
        if soname in runtime_sources:
            continue
        source = resolve_library(
            soname,
            consumer=binary,
            consumer_search_paths=(),
            cache=cache,
            machine=machine,
            library_directories=target.library_directories,
        )
        if source is None:
            continue
        runtime_sources[soname] = source
        queue.append(source)

    while queue:
        consumer = queue.pop(0).resolve()
        if consumer in scanned:
            continue
        scanned.add(consumer)
        if elf_machine(consumer) != machine:
            fail(f"runtime architecture mismatch: {consumer}")
        needed, search_paths = dynamic_metadata(consumer, target)
        for soname in needed:
            if is_host_driver(soname):
                fail(
                    f"{consumer} directly depends on host GPU driver {soname}; "
                    "portable packages may only use host drivers through runtime discovery"
                )
            source = resolve_library(
                soname,
                consumer=consumer,
                consumer_search_paths=search_paths,
                cache=cache,
                machine=machine,
                library_directories=target.library_directories,
            )
            if source is None:
                fail(f"missing required shared library {soname} for {consumer}")
            previous = runtime_sources.get(soname)
            if previous is not None and previous != source:
                if sha256(previous) != sha256(source):
                    fail(
                        f"{soname} resolves to conflicting files: {previous} and {source}"
                    )
            else:
                runtime_sources[soname] = source
            queue.append(source)

    for bundled_name, source in sorted(runtime_sources.items()):
        if elf_machine(source) != machine:
            fail(f"runtime architecture mismatch: {source}")
        copy_runtime_file(source, library_directory / bundled_name)

    libc_source = runtime_sources.get("libc.so.6")
    if libc_source is None:
        fail("recursive dependency closure did not contain libc.so.6")
    gconv_source = libc_source.parent / "gconv"
    license_sources = dict(runtime_sources)
    if gconv_source.is_dir():
        for source in sorted(gconv_source.rglob("*")):
            if source.is_file():
                relative = source.relative_to(gconv_source).as_posix()
                license_sources[f"gconv/{relative}"] = libc_source
        shutil.copytree(
            gconv_source,
            library_directory / "gconv",
            symlinks=False,
        )
    else:
        print(
            f"warning: glibc conversion modules were not found beside {libc_source}",
            file=sys.stderr,
        )

    documentation_directory.mkdir(parents=True, exist_ok=True)
    for license_name in ("LICENSE-APACHE", "NAVOP_LICENSE"):
        source = repository_root / license_name
        if not source.is_file():
            fail(f"project license file is missing: {source}")
        shutil.copy2(source, documentation_directory / license_name)

    package_records = copy_package_licenses(
        license_sources,
        runtime_license_directory,
    )
    packages_file = runtime_root / "runtime-packages.txt"
    packages_file.write_text(
        "".join(
            f"{record['package']}\t{record['version']}\t"
            f"{','.join(record['files'])}\n"
            for record in package_records
        ),
        encoding="utf-8",
    )

    launcher_machine = compile_launcher(
        launcher_source,
        launcher_destination,
        machine,
    )
    bundled_loader = library_directory / interpreter.name
    verify_private_runtime(
        bundled_loader,
        library_directory,
        binary_destination,
    )

    manifest_path = runtime_root / "runtime-manifest.json"
    manifest = {
        "schema_version": 1,
        "product": "navop",
        "target": args.target,
        "elf_machine": machine,
        "interpreter": str(interpreter),
        "launcher_machine": launcher_machine,
        "platform_token": target.platform_token,
        "lib_token": target.lib_token,
        "binary_glibc_baseline": args.glibc_baseline,
        "entrypoint": "usr/bin/navop",
        "binary": "usr/lib/navop/bin/navop.real",
        "loader": f"usr/lib/navop/lib/{interpreter.name}",
        "library_path": "usr/lib/navop/lib",
        "gpu_policy": (
            "host vendor libraries are discovered dynamically and are not bundled"
        ),
        "nss_policy": (
            "glibc NSS modules are bundled when available; host DNS, NSS, and "
            "certificate configuration remains authoritative"
        ),
        "packages": package_records,
        "host_interfaces": [
            "Linux kernel and procfs",
            "Wayland or X11 display sockets",
            "D-Bus session and system buses",
            "GPU devices and host vendor drivers",
            "system fonts and font configuration",
            "CA certificates, DNS, and NSS configuration",
        ],
        "files": manifest_files(output, manifest_path),
    }
    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

    print(
        f"Packaged {len(runtime_sources)} runtime ELF files into {output}",
    )


if __name__ == "__main__":
    main()
