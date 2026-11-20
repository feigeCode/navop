#!/usr/bin/env python3
"""下载并验证固定的 Windows X11 组件，供 Cargo 构建期内嵌；本脚本不接触私钥。"""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
RESOURCE_DIR = ROOT / "resources" / "windows" / "x11-component"
EXPECTED_COMPONENT_VERSION = "1.9.0-navop.1"
MAX_ARTIFACT_BYTES = 256 * 1024 * 1024
ED25519_SPKI_DER_PREFIX = bytes.fromhex("302a300506032b6570032100")


def prepare(target: str, output: Path) -> dict:
    manifest = json.loads((RESOURCE_DIR / "navop-x11-manifest.json").read_text("utf-8"))
    component = manifest["components"]["x11-server"]
    if manifest.get("schema") != 1 or component.get("channel") != "stable":
        raise ValueError("X11 清单 schema/channel 不受支持")
    if component.get("rollout_percent") != 100:
        raise ValueError("随包组件必须 rollout_percent=100")
    artifact = component["artifacts"][target]
    if artifact.get("version") != EXPECTED_COMPONENT_VERSION:
        raise ValueError("组件版本变化必须同时更新准备脚本的固定版本")
    executable_path = PurePosixPath(artifact.get("executable_path", ""))
    if (
        artifact.get("format") != "zip"
        or executable_path.is_absolute()
        or ".." in executable_path.parts
        or executable_path.name != "yserver.exe"
    ):
        raise ValueError("随包组件必须声明安全的 yserver.exe 相对路径")
    url = artifact["url"]
    if not url.startswith("https://"):
        raise ValueError("组件下载必须使用 HTTPS")
    expected_sha256 = artifact["sha256"].lower()
    if len(expected_sha256) != 64 or any(c not in "0123456789abcdef" for c in expected_sha256):
        raise ValueError("组件 SHA256 格式无效")
    signature = base64.b64decode(artifact["signature"], validate=True)
    public_key = base64.b64decode(
        (RESOURCE_DIR / "public-key.txt").read_text("ascii").strip(), validate=True
    )
    if len(signature) != 64 or len(public_key) != 32:
        raise ValueError("Ed25519 公钥或签名长度无效")

    output = output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    # Windows runner 的 TEMP 常在 C:，而 RUNNER_TEMP/输出在 D:；
    # 与输出同盘的 staging 才能保证 os.replace 原子提交不会 WinError 17。
    with tempfile.TemporaryDirectory(prefix=".navop-x11-prepare-", dir=output.parent) as temporary:
        temporary = Path(temporary)
        download = temporary / "component.zip"
        digest = hashlib.sha256()
        size = 0
        request = urllib.request.Request(url, headers={"User-Agent": "navop-x11-release-builder"})
        with urllib.request.urlopen(request, timeout=120) as response, download.open("wb") as writer:
            while chunk := response.read(1024 * 1024):
                size += len(chunk)
                if size > MAX_ARTIFACT_BYTES:
                    raise ValueError("组件超过 256 MiB 安全上限")
                digest.update(chunk)
                writer.write(chunk)
        actual_sha256 = digest.hexdigest()
        if size == 0 or actual_sha256 != expected_sha256:
            raise ValueError(f"组件 SHA256 不匹配：期望 {expected_sha256}，实际 {actual_sha256}")

        key_path = temporary / "public-key.der"
        signature_path = temporary / "signature.bin"
        key_path.write_bytes(ED25519_SPKI_DER_PREFIX + public_key)
        signature_path.write_bytes(signature)
        subprocess.run(
            [
                "openssl", "pkeyutl", "-verify", "-rawin",
                "-pubin", "-inkey", str(key_path), "-keyform", "DER",
                "-sigfile", str(signature_path), "-in", str(download),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        with zipfile.ZipFile(download) as archive:
            names = set(archive.namelist())
            root = executable_path.parent
            required_files = (
                str(executable_path),
                str(root / "xauth.exe"),
                str(root / "licenses" / "rust-crates.txt"),
            )
            missing = [name for name in required_files if name not in names]
            required_xkb_dirs = ("compat", "keycodes", "rules", "symbols", "types")
            if missing or any(
                not any(
                    name.startswith(f"{root}/xkb/{directory}/") and not name.endswith("/")
                    for name in names
                )
                for directory in required_xkb_dirs
            ):
                raise ValueError(f"组件包结构不完整：{missing} 或缺少 XKB 键盘数据")
        os.replace(download, output)

    if hashlib.sha256(output.read_bytes()).hexdigest() != expected_sha256:
        raise ValueError("输出组件复核失败")
    return {
        "target": target,
        "version": artifact["version"],
        "sha256": expected_sha256,
        "bytes": output.stat().st_size,
        "output": str(output),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=["windows-x86_64", "windows-aarch64"])
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(prepare(args.target, args.output), ensure_ascii=False))


if __name__ == "__main__":
    main()
