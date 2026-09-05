import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const read = (path) => fs.readFileSync(path, "utf8");

const workflowStep = (workflow, name) => {
  const marker = `      - name: ${name}`;
  const start = workflow.indexOf(marker);
  assert.ok(start >= 0, `missing workflow step: ${name}`);
  const end = workflow.indexOf("\n      - name:", start + marker.length);
  return workflow.slice(start, end >= 0 ? end : undefined);
};

test("release packaging uses the navop executable on every platform", () => {
  const release = read(".github/workflows/release.yml");
  const bundle = read("script/bundle-macos.sh");
  const plist = read("resources/macos/Info.plist");
  const desktop = read("resources/linux/navop.desktop");

  assert.doesNotMatch(release, /binary: onetcli(?:\.exe)?/);
  assert.match(release, /navop\.exe/);
  assert.match(bundle, /BINARY_NAME="navop"/);
  assert.doesNotMatch(bundle, /generate-macos-icon\.sh/);
  assert.match(bundle, /Error: Icon file not found/);
  assert.match(plist, /<key>CFBundleExecutable<\/key>\s*<string>navop<\/string>/);
  assert.match(desktop, /^Exec=navop %F$/m);
  assert.match(desktop, /^Icon=navop$/m);
  assert.match(desktop, /^StartupWMClass=navop$/m);
});

test("installers register database, Markdown, and terminal recording file associations", () => {
  const release = read(".github/workflows/release.yml");
  const plist = read("resources/macos/Info.plist");
  const desktop = read("resources/linux/navop.desktop");
  const wix = read("installer/windows/navop.wxs");
  const mimePath = "resources/linux/navop.xml";

  assert.match(plist, /<key>CFBundleDocumentTypes<\/key>/);
  for (const extension of ["db", "duckdb", "md", "cast"]) {
    assert.match(plist, new RegExp(`<string>${extension}<\\/string>`));
    assert.match(wix, new RegExp(`<Extension[^>]*Id="${extension}"`));
  }
  const macosRecordingDocument = plist.match(
    /<dict>\s*<key>CFBundleTypeName<\/key>\s*<string>Terminal Recording<\/string>[\s\S]*?<\/dict>/,
  )?.[0];
  assert.ok(macosRecordingDocument, "missing macOS terminal recording document type");
  assert.match(
    macosRecordingDocument,
    /<key>CFBundleTypeRole<\/key>\s*<string>Viewer<\/string>/,
  );
  assert.match(macosRecordingDocument, /<string>org\.asciinema\.cast<\/string>/);
  assert.match(macosRecordingDocument, /<string>cast<\/string>/);

  const macosRecordingUti = plist.match(
    /<dict>\s*<key>UTTypeIdentifier<\/key>\s*<string>org\.asciinema\.cast<\/string>[\s\S]*?<\/dict>/,
  )?.[0];
  assert.ok(macosRecordingUti, "missing macOS terminal recording UTI");
  assert.match(macosRecordingUti, /<string>public\.data<\/string>/);
  assert.match(
    macosRecordingUti,
    /<key>public\.filename-extension<\/key>\s*<array><string>cast<\/string><\/array>/,
  );
  assert.match(
    macosRecordingUti,
    /<key>public\.mime-type<\/key>\s*<string>application\/x-asciicast<\/string>/,
  );

  const windowsRecordingProgId = wix.match(
    /<ProgId[^>]*Id="Navop\.TerminalRecording"[\s\S]*?<\/ProgId>/,
  )?.[0];
  assert.ok(windowsRecordingProgId, "missing Windows terminal recording ProgId");
  assert.match(
    windowsRecordingProgId,
    /<Extension[^>]*Id="cast"[^>]*ContentType="application\/x-asciicast"/,
  );
  assert.doesNotMatch(plist, /<string>(?:cast\.)?partial<\/string>/);
  assert.doesNotMatch(wix, /<Extension[^>]*Id="(?:cast\.)?partial"/);

  assert.match(
    desktop,
    /^MimeType=.*application\/vnd\.sqlite3;.*application\/x-duckdb;.*text\/markdown;.*application\/x-asciicast;/m,
  );
  assert.ok(fs.existsSync(mimePath), `${mimePath} must exist`);
  const mime = read(mimePath);
  assert.match(mime, /type="application\/vnd\.sqlite3"/);
  assert.match(mime, /pattern="\*\.db"/);
  assert.match(mime, /type="application\/x-duckdb"/);
  assert.match(mime, /pattern="\*\.duckdb"/);
  assert.match(mime, /type="text\/markdown"/);
  assert.match(mime, /pattern="\*\.md"/);
  assert.match(mime, /type="application\/x-asciicast"/);
  assert.match(mime, /pattern="\*\.cast"/);
  assert.match(mime, /pattern="\*\.cast\.partial"/);
  assert.doesNotMatch(mime, /pattern="\*\.partial"/);
  assert.match(release, /package\/usr\/share\/mime\/packages/);
  assert.match(release, /resources\/linux\/navop\.xml/);
  assert.match(release, /\/usr\/share\/mime\/packages\/navop\.xml/);
  assert.match(release, /update-mime-database \/usr\/share\/mime/);
  assert.match(release, /update-desktop-database \/usr\/share\/applications/);
});

test("renamed Linux packages replace legacy onetcli installations", () => {
  const release = read(".github/workflows/release.yml");

  assert.match(release, /Package: navop/);
  assert.match(release, /Provides: onetcli/);
  assert.match(release, /Replaces: onetcli/);
  assert.match(release, /Conflicts: onetcli/);
  assert.match(release, /Name: navop/);
  assert.match(release, /Obsoletes: onetcli/);
});

test("Linux keeps full-feature standard packages and publishes portable variants separately", () => {
  const release = read(".github/workflows/release.yml");
  const installZig = workflowStep(
    release,
    "Install Zig toolchain (portable Linux)",
  );
  const installPortable = workflowStep(
    release,
    "Install portable packaging dependencies",
  );
  const build = workflowStep(release, "Build release binary");
  const verifyPortable = workflowStep(
    release,
    "Verify portable Linux glibc baseline",
  );
  const packageLinux = workflowStep(release, "Package (Linux)");
  const packageInstallers = workflowStep(
    release,
    "Package Linux installers (x86_64)",
  );

  assert.match(
    release,
    /linux_x64='\{"target":"x86_64-unknown-linux-gnu","os":"ubuntu-latest"[^']*"archive":"navop-x86_64-unknown-linux-gnu\.tar\.gz"[^']*"variant":"standard"[^']*"portable_linux":false\}'/,
  );
  assert.match(
    release,
    /linux_x64_portable='\{"target":"x86_64-unknown-linux-gnu","os":"ubuntu-22\.04"[^']*"archive":"navop-x86_64-unknown-linux-gnu-portable\.tar\.gz"[^']*"variant":"portable"[^']*"portable_linux":true\}'/,
  );
  assert.match(
    release,
    /linux_arm64='\{"target":"aarch64-unknown-linux-gnu"[^']*"archive":"navop-aarch64-unknown-linux-gnu\.tar\.gz"[^']*"variant":"standard"[^']*"portable_linux":false\}'/,
  );
  assert.match(
    release,
    /linux_arm64_portable='\{"target":"aarch64-unknown-linux-gnu"[^']*"archive":"navop-aarch64-unknown-linux-gnu-portable\.tar\.gz"[^']*"variant":"portable"[^']*"portable_linux":true\}'/,
  );
  assert.match(
    release,
    /all\) matrix="\[\$macos_arm64,\$macos_x64,\$linux_x64,\$linux_x64_portable,\$linux_arm64,\$linux_arm64_portable,\$windows_x64,\$windows_x86\]"/,
  );
  assert.match(
    release,
    /linux-x64\) matrix="\[\$linux_x64,\$linux_x64_portable\]"/,
  );
  assert.match(
    release,
    /linux-x64-portable\) matrix="\[\$linux_x64_portable\]"/,
  );
  assert.match(
    release,
    /linux-arm64\) matrix="\[\$linux_arm64,\$linux_arm64_portable\]"/,
  );
  assert.match(
    release,
    /linux-arm64-portable\) matrix="\[\$linux_arm64_portable\]"/,
  );
  assert.match(installZig, /if: matrix\.portable_linux/);
  assert.match(installZig, /python3 -m venv "\$RUNNER_TEMP\/ziglang"/);
  assert.match(installZig, /ziglang==0\.14\.1/);
  assert.match(
    installZig,
    /cargo install --locked cargo-zigbuild --version 0\.23\.0/,
  );
  assert.match(
    installZig,
    /CARGO_ZIGBUILD_PYTHON_PATH=\$RUNNER_TEMP\/ziglang\/bin\/python/,
  );
  assert.match(installZig, /cargo-zigbuild --version/);
  assert.doesNotMatch(installZig, /cargo zigbuild --version/);
  assert.match(installPortable, /if: matrix\.portable_linux/);
  assert.match(installPortable, /apt-get install -y binutils musl-tools/);
  assert.match(build, /if \[ "\$\{\{ matrix\.portable_linux \}\}" = "true" \]/);
  assert.match(
    build,
    /cargo zigbuild[\s\S]*--release[\s\S]*-p main[\s\S]*--target "\$\{\{ matrix\.target \}\}\.2\.28"[\s\S]*--no-default-features[\s\S]*--features wasm-components/,
  );
  assert.match(
    build,
    /cargo build --release -p main --target "\$\{\{ matrix\.target \}\}"/,
  );
  assert.match(
    build,
    /test -x "target\/\$\{\{ matrix\.target \}\}\/release\/\$\{\{ matrix\.binary \}\}"/,
  );
  assert.match(verifyPortable, /if: matrix\.portable_linux/);
  assert.match(
    verifyPortable,
    /script\/check-linux-glibc-baseline\.sh[\s\S]*target\/\$\{\{ matrix\.target \}\}\/release\/\$\{\{ matrix\.binary \}\}[\s\S]*2\.28/,
  );
  assert.match(
    packageLinux,
    /if \[ "\$\{\{ matrix\.portable_linux \}\}" = "true" \]; then[\s\S]*script\/package-linux-portable\.sh/,
  );
  assert.match(
    packageLinux,
    /else[\s\S]*cp "target\/\$\{\{ matrix\.target \}\}\/release\/\$\{\{ matrix\.binary \}\}" package\/usr\/bin\//,
  );
  assert.match(packageLinux, /--sort=name/);
  assert.match(packageLinux, /--numeric-owner/);
  assert.match(
    packageInstallers,
    /if: matrix\.target == 'x86_64-unknown-linux-gnu' && !matrix\.portable_linux/,
  );
});

test("portable Linux disables WebView while standard builds keep it", () => {
  const workspaceCargo = read("Cargo.toml");
  const cargo = read("crates/ai_chat_view/Cargo.toml");
  const mainCargo = read("main/Cargo.toml");
  const htmlCodeBlock = read(
    "crates/ai_chat_view/src/html_code_block.rs",
  );
  const dependentCargoFiles = [
    "main/Cargo.toml",
    "crates/db_view/Cargo.toml",
    "crates/mongodb_view/Cargo.toml",
    "crates/redis_view/Cargo.toml",
    "crates/terminal_view/Cargo.toml",
  ];

  assert.match(cargo, /default = \["embedded-webview"\]/);
  assert.match(
    cargo,
    /embedded-webview = \["dep:gpui-wry", "dep:wry"\]/,
  );
  assert.match(cargo, /gpui-wry = \{[^}]*optional = true[^}]*\}/);
  assert.match(cargo, /wry = \{[^}]*optional = true[^}]*\}/);
  assert.doesNotMatch(cargo, /target_arch = "aarch64"/);
  assert.match(
    htmlCodeBlock,
    /cfg\(feature = "embedded-webview"\)/,
  );
  assert.match(
    htmlCodeBlock,
    /cfg\(not\(feature = "embedded-webview"\)\)[\s\S]*?fn refresh_webview/,
  );
  assert.doesNotMatch(htmlCodeBlock, /target_arch = "aarch64"/);
  assert.match(htmlCodeBlock, /HtmlPreview\.webview_unavailable/);
  assert.match(
    mainCargo,
    /default = \["wasm-components", "embedded-webview", "windows-native-rdp", "builtin-mqtt"\]/,
  );
  assert.match(
    mainCargo,
    /embedded-webview = \["ai_chat_view\/embedded-webview"\]/,
  );
  assert.match(
    mainCargo,
    /gpui-shell = \{ workspace = true, optional = true \}/,
  );
  assert.match(
    mainCargo,
    /gpui-component-shell = \{ workspace = true, optional = true \}/,
  );
  assert.match(
    mainCargo,
    /shell-plugins = \["dep:gpui-shell", "dep:gpui-component-shell"\]/,
  );
  assert.match(
    workspaceCargo,
    /ai_chat_view = \{ path = "crates\/ai_chat_view", default-features = false \}/,
  );
  for (const manifest of dependentCargoFiles) {
    assert.match(
      read(manifest),
      /ai_chat_view = \{ workspace = true, default-features = false \}/,
      `${manifest} must not implicitly enable ai_chat_view defaults`,
    );
  }
});

test("Linux portable packager uses a private loader and recursive ELF closure", () => {
  const wrapperPath = "script/package-linux-portable.sh";
  const packagerPath = "script/package-linux-portable.py";
  const launcherPath = "script/linux-portable-launcher.c";

  for (const file of [wrapperPath, packagerPath, launcherPath]) {
    assert.ok(fs.existsSync(file), `${file} must exist`);
  }

  const wrapper = read(wrapperPath);
  const packager = read(packagerPath);
  const launcher = read(launcherPath);
  const help = spawnSync("python3", [packagerPath, "--help"], {
    encoding: "utf8",
  });

  assert.equal(help.status, 0, help.stderr);
  assert.match(wrapper, /set -euo pipefail/);
  assert.match(wrapper, /package-linux-portable\.py/);
  assert.match(packager, /readelf/);
  assert.match(packager, /PT_INTERP/);
  assert.match(packager, /verify_binary_glibc_baseline/);
  assert.match(packager, /binary_glibc_baseline/);
  assert.match(
    packager,
    /the bundled "[\s\S]*"private runtime itself may be newer/,
  );
  assert.match(packager, /\\\(NEEDED\\\)/);
  assert.match(packager, /ldconfig/);
  assert.match(packager, /dpkg-query/);
  assert.match(packager, /musl-gcc/);
  assert.match(packager, /"-static"/);
  assert.match(packager, /runtime-manifest\.json/);
  assert.match(packager, /runtime-packages\.txt/);
  assert.match(packager, /runtime-licenses/);
  assert.match(packager, /LICENSE-APACHE/);
  assert.match(packager, /NAVOP_LICENSE/);
  assert.match(packager, /libnss_dns\.so\.2/);
  assert.match(packager, /libnss_files\.so\.2/);
  assert.match(packager, /libnss_\*\.so\.2/);
  assert.match(packager, /libwayland-client\.so\.0/);
  assert.match(packager, /libwayland-cursor\.so\.0/);
  assert.match(packager, /libwayland-egl\.so\.1/);
  assert.match(packager, /missing required dlopen runtime library/);
  assert.match(packager, /libvulkan_\*\.so\*/);
  assert.match(help.stdout, /aarch64-unknown-linux-gnu/);
  assert.match(help.stdout, /x86_64-unknown-linux-gnu/);
  assert.match(packager, /machine="AArch64"/);
  assert.match(packager, /loader="ld-linux-aarch64\.so\.1"/);
  assert.match(packager, /platform_token="aarch64"/);
  assert.match(packager, /lib_token="lib"/);
  assert.match(packager, /machine="Advanced Micro Devices X86-64"/);
  assert.match(packager, /loader="ld-linux-x86-64\.so\.2"/);
  assert.match(packager, /platform_token="x86_64"/);
  assert.match(packager, /lib_token="lib64"/);
  assert.match(packager, /launcher architecture mismatch/);
  assert.match(packager, /launcher_machine/);
  assert.match(packager, /gpu_policy/);
  assert.match(packager, /nss_policy/);
  assert.match(packager, /license_sources\[f"gconv\/\{relative\}"\]/);
  assert.match(
    packager,
    /cannot publish a bundled runtime file without Debian package[\s\S]*ownership metadata/,
  );
  assert.match(
    packager,
    /cannot publish bundled runtime package without its copyright[\s\S]*file/,
  );
  assert.match(launcher, /\/proc\/self\/exe/);
  assert.match(launcher, /defined\(__aarch64__\)/);
  assert.match(launcher, /defined\(__x86_64__\)/);
  assert.match(launcher, /ld-linux-aarch64\.so\.1/);
  assert.match(launcher, /ld-linux-x86-64\.so\.2/);
  assert.match(launcher, /NAVOP_PORTABLE_LOADER/);
  assert.match(launcher, /--inhibit-cache/);
  assert.match(launcher, /--library-path/);
  assert.match(launcher, /navop\.real/);
  assert.match(launcher, /GCONV_PATH/);
  assert.match(launcher, /unsetenv\("LD_PRELOAD"\)/);
  assert.match(launcher, /unsetenv\("GLIBC_TUNABLES"\)/);
});

test("Linux portable packager resolves Debian ownership across usrmerge aliases", () => {
  const packagerPath = path.resolve("script/package-linux-portable.py");
  const python = String.raw`
import importlib.util
from pathlib import Path
import subprocess
import sys

spec = importlib.util.spec_from_file_location("navop_portable_packager", sys.argv[1])
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)

loader_in_usr = Path("/usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2")
loader_in_lib = Path("/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2")

usr_candidates = module.package_owner_query_paths(loader_in_usr)
lib_candidates = module.package_owner_query_paths(loader_in_lib)
assert loader_in_usr in usr_candidates
assert loader_in_lib in usr_candidates
assert loader_in_lib in lib_candidates
assert loader_in_usr in lib_candidates

queries = []
def fake_run(command, *, check=True, env=None):
    queries.append(command)
    if command == ["dpkg-query", "-S", str(loader_in_lib)]:
        return subprocess.CompletedProcess(
            command,
            0,
            stdout=f"libc6:amd64: {loader_in_lib}\n",
            stderr="",
        )
    return subprocess.CompletedProcess(command, 1, stdout="", stderr="")

module.run = fake_run
assert module.package_owner(loader_in_usr) == "libc6:amd64"
assert ["dpkg-query", "-S", str(loader_in_lib)] in queries
`;
  const result = spawnSync("python3", ["-c", python, packagerPath], {
    encoding: "utf8",
  });

  assert.equal(result.status, 0, result.stderr || result.stdout);
});

test("glibc baseline checker rejects binaries above the configured version", () => {
  const checker = "script/check-linux-glibc-baseline.sh";
  assert.ok(fs.existsSync(checker), `${checker} must exist`);

  const fixtureDir = fs.mkdtempSync(
    path.join(os.tmpdir(), "navop-glibc-check-"),
  );
  const fakeReadelf = path.join(fixtureDir, "readelf");
  const binary = path.join(fixtureDir, "navop");
  fs.writeFileSync(binary, "");

  const runChecker = (readelfOutput) => {
    fs.writeFileSync(
      fakeReadelf,
      `#!/usr/bin/env bash\ncat <<'EOF'\n${readelfOutput}\nEOF\n`,
      { mode: 0o755 },
    );
    return spawnSync("bash", [checker, binary, "2.28"], {
      encoding: "utf8",
      env: { ...process.env, READELF: fakeReadelf },
    });
  };

  try {
    const compatible = runChecker(
      "Name: GLIBC_2.17\nName: GLIBC_2.28",
    );
    assert.equal(compatible.status, 0, compatible.stderr);
    assert.match(compatible.stdout, /highest required GLIBC version: 2\.28/);

    const incompatible = runChecker(
      "Name: GLIBC_2.17\nName: GLIBC_2.29",
    );
    assert.notEqual(incompatible.status, 0);
    assert.match(incompatible.stderr, /requires GLIBC_2\.29/);

    const missingSymbols = runChecker("No version information found");
    assert.notEqual(missingSymbols.status, 0);
    assert.match(missingSymbols.stderr, /did not report any GLIBC versions/);
  } finally {
    fs.rmSync(fixtureDir, { recursive: true, force: true });
  }
});

test("Windows release builds an installable per-user MSI", () => {
  const release = read(".github/workflows/release.yml");
  const wix = read("installer/windows/navop.wxs");

  assert.match(release, /dotnet tool install --global wix --version 6\.0\.2/);
  assert.match(release, /wix build installer\/windows\/navop\.wxs/);
  assert.match(
    release,
    /-out "\$\{env:PUBLIC_BASENAME\}\.msi"/,
  );
  assert.match(wix, /Scope="perUser"/);
  assert.match(wix, /StandardDirectory Id="LocalAppDataFolder"/);
  assert.match(wix, /<File[^>]+Source="\$\(SourceDir\)\\navop\.exe"/);
  assert.match(wix, /MajorUpgrade/);
  assert.match(wix, /ProgramMenuFolder/);
  assert.match(wix, /Shortcut[^]*Name="Navop"/);
  assert.match(wix, /RemoveFolder[^]*On="uninstall"/);
  assert.match(wix, /Root="HKCU"/);
});

test("Windows application builds include the native RDP backend", () => {
  const release = read(".github/workflows/release.yml");
  const releaseWindowsBuild = release.match(
    /- name: Build release binary \(Windows\)[\s\S]*?(?=\n      - name:)/,
  )?.[0];

  assert.ok(releaseWindowsBuild, "missing Windows release binary build step");
  for (const target of [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
  ]) {
    assert.match(
      release,
      new RegExp(
        `"target":"${target}"[^']*"windows_native_rdp":false`,
      ),
    );
  }
  for (const target of [
    "x86_64-pc-windows-msvc",
    "i686-pc-windows-msvc",
  ]) {
    assert.match(
      release,
      new RegExp(
        `"target":"${target}"[^']*"windows_native_rdp":true`,
      ),
    );
  }
  assert.match(
    releaseWindowsBuild,
    /if: runner\.os == 'Windows'/,
  );
  assert.match(
    releaseWindowsBuild,
    /--features windows-native-rdp/,
  );
  assert.match(
    releaseWindowsBuild,
    /--target \$target/,
  );
  assert.match(
    release,
    /- name: Configure MSVC environment[\s\S]*?if: runner\.os == 'Windows'[\s\S]*?uses: ilammy\/msvc-dev-cmd@v1[\s\S]*?arch: \$\{\{ matrix\.windows_arch \}\}/,
  );
  assert.match(releaseWindowsBuild, /VCToolsInstallDir/);
});

test("Windows release publishes versioned Win32 artifacts while preserving updater metadata", () => {
  const release = read(".github/workflows/release.yml");
  const upload = read(".github/workflows/upload-r2.yml");
  const cargoConfig = read(".cargo/config.toml");

  assert.match(release, /- windows-x86/);
  assert.match(
    release,
    /windows_x86='\{"target":"i686-pc-windows-msvc"[^']*"archive":"navop-i686-pc-windows-msvc\.zip"[^']*"public_label":"win32"[^']*"windows_arch":"x86"/,
  );
  assert.match(
    release,
    /all\) matrix="\[\$macos_arm64,\$macos_x64,\$linux_x64,\$linux_x64_portable,\$linux_arm64,\$linux_arm64_portable,\$windows_x64,\$windows_x86\]"/,
  );
  assert.match(
    release,
    /\$\{env:PUBLIC_BASENAME\}-portable\.zip/,
  );
  assert.match(release, /-arch \$\{\{ matrix\.windows_arch \}\}/);
  assert.match(release, /\$\{env:PUBLIC_BASENAME\}\.msi/);
  assert.match(release, /\$\{env:PUBLIC_BASENAME\}\.exe/);
  assert.match(release, /PUBLIC_BASENAME=navop-\$\{VERSION#v\}-\$\{\{ matrix\.public_label \}\}/);

  assert.match(upload, /navop-i686-pc-windows-msvc\.zip/);
  assert.match(
    upload,
    /"i686-pc-windows-msvc": "navop-i686-pc-windows-msvc\.zip"/,
  );
  assert.match(
    cargoConfig,
    /\[target\.i686-pc-windows-msvc\][\s\S]*?link-arg=\/STACK:8000000/,
  );
});

test("Windows release builds an EXE installer bundle from the MSI", () => {
  const bundlePath = "installer/windows/navop-bundle.wxs";
  assert.ok(fs.existsSync(bundlePath), `${bundlePath} must exist`);

  const bundle = read(bundlePath);
  const release = read(".github/workflows/release.yml");

  assert.match(
    bundle,
    /xmlns:bal="http:\/\/wixtoolset\.org\/schemas\/v4\/wxs\/bal"/,
  );
  assert.match(bundle, /<Bundle[^>]*Id="feigeCode\.Navop"/);
  assert.doesNotMatch(bundle, /UpgradeCode=/);
  assert.match(bundle, /<bal:WixInternalUIBootstrapperApplication\s*\/>/);
  assert.match(
    bundle,
    /<MsiPackage[^>]*SourceFile="\$\(MsiPath\)"[^>]*Compressed="yes"[^>]*bal:PrimaryPackageType="default"/,
  );
  assert.doesNotMatch(bundle, /bal:PrimaryPackageType="x64"/);

  assert.match(
    release,
    /WixToolset\.BootstrapperApplications\.wixext\/6\.0\.2/,
  );
  assert.match(
    release,
    /wix build installer\/windows\/navop-bundle\.wxs[^]*-ext WixToolset\.BootstrapperApplications\.wixext[^]*-d Version=[^\n]+[^]*-d MsiPath=[^\n]*\$\{env:PUBLIC_BASENAME\}\.msi[^]*-out "\$\{env:PUBLIC_BASENAME\}\.exe"/,
  );
  assert.doesNotMatch(
    release,
    /Copy-Item[^\n]+"navop-x86_64-pc-windows-msvc\.exe"/,
  );

  const msiBuild = release.indexOf(
    "wix build installer/windows/navop.wxs",
  );
  const bundleBuild = release.indexOf(
    "wix build installer/windows/navop-bundle.wxs",
  );
  assert.ok(msiBuild >= 0, "missing MSI build");
  assert.ok(bundleBuild > msiBuild, "EXE installer must be built after MSI");
});

test("Windows release keeps the legacy ZIP standard and publishes portable separately", () => {
  const release = read(".github/workflows/release.yml");
  const installGuides = [
    read("docs-site/docs/guide/install-update.md"),
    read("docs-site/docs/en-US/guide/install-update.md"),
    read("docs-site/docs/zh-TW/guide/install-update.md"),
  ];

  assert.match(release, /portable-package/);
  assert.match(release, /navop\.portable/);
  assert.match(
    release,
    /Set-Content -Path "portable-package\/navop\.portable"/,
  );
  assert.doesNotMatch(
    release,
    /"package\/navop\.portable"/,
  );
  assert.match(release, /-d SourceDir=.*\\package/);
  assert.doesNotMatch(release, /-d SourceDir=.*portable-package/);
  assert.match(
    release,
    /Compress-Archive -Path "package\/\*" -DestinationPath "\$\{\{ matrix\.archive \}\}"/,
  );
  assert.match(
    release,
    /Compress-Archive -Path "portable-package\/\*" -DestinationPath "\$\{env:PUBLIC_BASENAME\}-portable\.zip"/,
  );
  assert.match(release, /navop-x86_64-pc-windows-msvc\.zip/);
  assert.match(
    release,
    /windows_x64='\{"target":"x86_64-pc-windows-msvc"[^']*"archive":"navop-x86_64-pc-windows-msvc\.zip"/,
  );
  assert.match(
    release,
    /name: navop-\$\{\{ matrix\.public_label \}\}-packages/,
  );
  assert.match(
    release,
    /navop-\*-\$\{\{ matrix\.public_label \}\}-portable\.zip/,
  );
  assert.match(
    release,
    /navop-\*-\$\{\{ matrix\.public_label \}\}\.msi/,
  );
  assert.match(
    release,
    /navop-\*-\$\{\{ matrix\.public_label \}\}\.exe/,
  );
  for (const [guide, installerLabel] of [
    [installGuides[0], /EXE 安装包/],
    [installGuides[1], /EXE installer/],
    [installGuides[2], /EXE 安裝包/],
  ]) {
    assert.match(guide, /navop-<version>-windows-x64\.exe/);
    assert.match(guide, installerLabel);
    assert.match(guide, /-portable\.zip/);
    assert.doesNotMatch(
      guide,
      /(?:独立 EXE|獨立 EXE|standalone EXE|standalone \.exe|官方 Windows ZIP 已包含|官方 Windows \.zip 是便携版|The official Windows ZIP already includes|The official Windows \.zip is the portable edition|官方 Windows ZIP 已包含|官方 Windows \.zip 是便攜版)/,
    );
  }
});

test("Windows MSI appends Navop to the directory chosen by users", () => {
  const release = read(".github/workflows/release.yml");
  const wix = read("installer/windows/navop.wxs");

  assert.match(
    release,
    /wix extension add -g WixToolset\.UI\.wixext\/6\.0\.2/,
  );
  assert.match(
    release,
    /wix build installer\/windows\/navop\.wxs[^]*-ext WixToolset\.UI\.wixext/,
  );
  assert.match(
    wix,
    /xmlns:ui="http:\/\/wixtoolset\.org\/schemas\/v4\/wxs\/ui"/,
  );
  assert.match(
    wix,
    /<ui:WixUI[^>]*Id="WixUI_InstallDir"[^>]*InstallDirectory="INSTALLROOT"/,
  );
  assert.match(
    wix,
    /<Directory Id="INSTALLROOT" Name="Programs">\s*<Directory Id="INSTALLFOLDER" Name="Navop"/,
  );
  assert.doesNotMatch(wix, /InstallDirectory="INSTALLFOLDER"/);
});

test("Windows MSI builds one bilingual localized installer", () => {
  const release = read(".github/workflows/release.yml");
  const wix = read("installer/windows/navop.wxs");
  const localizationPath = "installer/windows/navop.wxl";
  const licensePath = "installer/windows/navop-license.rtf";

  assert.match(wix, /Language="1033"/);
  assert.match(wix, /Codepage="936"/);
  assert.match(wix, /WixUILicenseRtf[^]*navop-license\.rtf/);
  assert.match(release, /node script\/generate-windows-license\.mjs/);
  assert.match(release, /-culture en-US/);
  assert.match(release, /-loc installer\/windows\/navop\.wxl/);
  assert.equal(
    (release.match(/wix build installer\/windows\/navop\.wxs/g) ?? [])
      .length,
    1,
  );
  assert.doesNotMatch(release, /navop-x86_64-pc-windows-msvc-zh-CN\.msi/);

  assert.ok(fs.existsSync(localizationPath), `${localizationPath} must exist`);
  const localization = read(localizationPath);
  assert.match(localization, /Estimated time remaining/);
  assert.match(localization, /预计剩余时间/);
  assert.match(localization, /I have read and accept/);
  assert.match(localization, /我已阅读并同意/);

  assert.ok(fs.existsSync(licensePath), `${licensePath} must exist`);
  const license = read(licensePath);
  assert.match(license, /Apache License/);
  assert.match(license, /Navop Software License Agreement/);
  assert.match(license, /\\u/);
  assert.doesNotMatch(license, /Lorem ipsum/);
});

test("Windows MSI creates a desktop shortcut", () => {
  const wix = read("installer/windows/navop.wxs");

  assert.match(wix, /<StandardDirectory Id="DesktopFolder"\s*\/>/);
  assert.match(
    wix,
    /<Shortcut[^>]*Id="DesktopShortcut"[^>]*Name="Navop"/,
  );
});

test("Windows MSI shortcuts use dedicated HKCU-keyed components", () => {
  const wix = read("installer/windows/navop.wxs");
  const component = (id) => {
    const match = wix.match(
      new RegExp(`<Component\\s+Id="${id}"[^>]*>([\\s\\S]*?)<\\/Component>`),
    );
    assert.ok(match, `missing ${id} component`);
    return match[0];
  };

  const executable = component("ApplicationExecutable");
  assert.doesNotMatch(executable, /<Shortcut\b/);

  for (const [componentId, directory, shortcutId, registryName] of [
    [
      "StartMenuShortcutComponent",
      "ApplicationProgramsFolder",
      "StartMenuShortcut",
      "StartMenuShortcutInstalled",
    ],
    [
      "DesktopShortcutComponent",
      "DesktopFolder",
      "DesktopShortcut",
      "DesktopShortcutInstalled",
    ],
  ]) {
    const shortcutComponent = component(componentId);
    assert.match(
      shortcutComponent,
      new RegExp(`<Component[^>]*Directory="${directory}"`),
    );
    assert.match(
      shortcutComponent,
      new RegExp(
        `<Shortcut[^>]*Id="${shortcutId}"[^>]*Target="\\[#NavopExecutable\\]"[^>]*Advertise="no"`,
      ),
    );
    assert.match(
      shortcutComponent,
      new RegExp(
        `<RegistryValue[^>]*Root="HKCU"[^>]*Name="${registryName}"[^>]*KeyPath="yes"`,
      ),
    );
  }
});

test("GitHub and R2 publish every installer while the updater manifest remains compatible", () => {
  const release = read(".github/workflows/release.yml");
  const upload = read(".github/workflows/upload-r2.yml");

  assert.match(
    release,
    /name: navop-\$\{\{ matrix\.public_label \}\}-packages[\s\S]*?navop-\*-\$\{\{ matrix\.public_label \}\}\.msi/,
  );
  assert.match(release, /new_files=\(artifacts\/navop-\* artifacts\/navop_\*\)/);
  assert.match(release, /navop-aarch64-unknown-linux-gnu-portable\.tar\.gz/);
  assert.match(release, /navop-x86_64-unknown-linux-gnu-portable\.tar\.gz/);
  assert.match(upload, /--pattern "navop-\*"/);
  assert.match(upload, /--pattern "navop_\*"/);
  assert.match(upload, /release_files=\(artifacts\/navop-\* artifacts\/navop_\*\)/);
  assert.match(upload, /schema_version: 1/);
  assert.match(upload, /downloads: objectUrls\("releases", updaterAssets\)/);
  assert.match(upload, /fallback_downloads: githubReleaseUrls\(updaterAssets\)/);
  assert.match(upload, /sha256s: objectChecksums\(updaterAssets\)/);
  assert.match(upload, /packages,/);
  assert.match(upload, /publicUpdaterAlternatives/);
  assert.match(upload, /`navop-\$\{version\}-win32\.zip`/);
  assert.match(upload, /\["win32", "i686-pc-windows-msvc"\]/);
  assert.match(upload, /\*\.dmg\) content_type="application\/x-apple-diskimage"/);
  assert.match(upload, /\*\.msi\) content_type="application\/x-msi"/);
  assert.match(upload, /\*\.exe\) content_type="application\/vnd\.microsoft\.portable-executable"/);
  assert.match(upload, /\*\.deb\) content_type="application\/vnd\.debian\.binary-package"/);
  assert.match(upload, /\*\.rpm\) content_type="application\/x-rpm"/);
  assert.match(upload, /\*\.AppImage\) content_type="application\/octet-stream"/);
});

test("R2 uploads are single-dispatch, revalidated, and verified after overwrite", () => {
  const upload = read(".github/workflows/upload-r2.yml");

  assert.match(upload, /workflow_dispatch:/);
  assert.doesNotMatch(upload, /workflow_run:/);
  assert.match(upload, /group: \$\{\{ github\.workflow \}\}-\$\{\{ inputs\.tag \}\}/);
  assert.match(upload, /cancel-in-progress: false/);
  assert.match(upload, /--metadata "sha256=\$\{expected_sha256\}"/);
  assert.match(upload, /aws s3api head-object/);
  assert.match(upload, /R2 object size mismatch/);
  assert.match(upload, /R2 object checksum metadata mismatch/);
  assert.match(upload, /public, max-age=0, must-revalidate/);
  assert.match(upload, /no-store, max-age=0/);
  assert.doesNotMatch(upload, /max-age=31536000/);
  assert.doesNotMatch(upload, /max-age=31536000, immutable/);
});

test("CNB release synchronization replaces moved tags before syncing assets", () => {
  const sync = read(".github/workflows/sync-cnb-release-assets.yml");

  assert.match(sync, /uses: actions\/checkout@v4/);
  assert.match(sync, /fetch-depth: 0/);
  assert.match(sync, /ref: \$\{\{ inputs\.tag \}\}/);
  assert.match(sync, /group: navop-cnb-release/);
  assert.match(sync, /cancel-in-progress: false/);
  assert.match(sync, /git remote add cnb "https:\/\/cnb\.cool\/\$\{CNB_REPOSITORY\}\.git"/);
  assert.match(sync, /git ls-remote --tags cnb/);
  assert.match(sync, /git for-each-ref --format='%\(refname\)' refs\/tags/);
  assert.match(sync, /git push cnb ":refs\/tags\/\$\{tag_name\}"/);
  assert.match(sync, /git push cnb --tags/);
  assert.match(sync, /\.\/mpgrm releases sync/);

  const deleteMovedTag = sync.indexOf('git push cnb ":refs/tags/${tag_name}"');
  const pushTags = sync.indexOf("git push cnb --tags");
  const syncAssets = sync.indexOf("./mpgrm releases sync");
  assert.ok(deleteMovedTag >= 0 && deleteMovedTag < pushTags);
  assert.ok(pushTags >= 0 && pushTags < syncAssets);
});

test("CI runs release packaging regression checks", () => {
  const ci = read(".github/workflows/ci.yml");

  assert.match(ci, /node --test script\/test-release-packaging\.mjs/);
  assert.match(ci, /workflow_dispatch:/);
  assert.match(ci, /- windows/);
  assert.match(ci, /fromJSON\(needs\.prepare\.outputs\.matrix\)/);
});

test("Windows release validates the MSI installer with the shared validator", () => {
  const release = read(".github/workflows/release.yml");
  const validatorPath = "script/validate-windows-msi.ps1";
  assert.ok(fs.existsSync(validatorPath), `${validatorPath} must exist`);
  assert.match(release, /validate-windows-msi\.ps1/);

  const validator = read(validatorPath);
  assert.match(validator, /ProductLanguage/);
  assert.match(validator, /WIXUI_INSTALLDIR/);
  assert.match(validator, /DesktopShortcut/);
  assert.match(validator, /StartMenuShortcut/);
  assert.match(validator, /DesktopShortcutComponent/);
  assert.match(validator, /StartMenuShortcutComponent/);
  assert.match(validator, /DesktopShortcutRegistry/);
  assert.match(validator, /StartMenuShortcutRegistry/);
  assert.match(validator, /SELECT Component_ FROM Shortcut/);
  assert.match(validator, /SELECT KeyPath FROM Component/);
  assert.match(validator, /SELECT Root FROM Registry/);
  assert.match(validator, /\.Trim\(\)/);
  assert.match(validator, /\$null = \$view\.Execute\(\)/);
  assert.match(validator, /\$null = \$view\.Close\(\)/);
  assert.match(validator, /\$value = \[string\]\$record\.StringData\(1\)/);
});

test("release builds use parallel thin LTO with a longer macOS ARM timeout", () => {
  const release = read(".github/workflows/release.yml");
  assert.doesNotMatch(release, /^\s+CARGO_PROFILE_RELEASE_LTO:\s/m);
  assert.doesNotMatch(release, /^\s+CARGO_PROFILE_RELEASE_CODEGEN_UNITS:\s/m);
  assert.match(release, /export CARGO_PROFILE_RELEASE_LTO=thin/);
  assert.match(release, /export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16/);
  assert.match(
    release,
    /timeout-minutes: \$\{\{ matrix\.target == 'aarch64-apple-darwin' && 180 \|\| matrix\.arm_linux && 180 \|\| 120 \}\}/,
  );

  const cargo = read("Cargo.toml");
  assert.match(cargo, /\[profile\.release\][\s\S]*?lto = "thin"/);
  assert.match(cargo, /\[profile\.release\][\s\S]*?codegen-units = 16/);
});

test("release builds are cacheable and individually repairable", () => {
  const release = read(".github/workflows/release.yml");
  const trigger = read(".github/workflows/release-trigger.yml");

  for (const platform of [
    "macos-arm64",
    "macos-x64",
    "linux-x64",
    "linux-x64-portable",
    "linux-arm64",
    "linux-arm64-portable",
    "windows-x64",
    "windows-x86",
  ]) {
    assert.match(release, new RegExp(`- ${platform}`));
  }
  assert.match(release, /mozilla-actions\/sccache-action@v0\.0\.10/);
  assert.match(release, /SCCACHE_GHA_ENABLED: "true"/);
  assert.match(release, /navop-cargo-inputs-v1-/);
  assert.match(release, /cache: false/);
  assert.doesNotMatch(release, /release-cargo-[^\n]*github\.run_id/);
  assert.match(release, /No existing release assets found/);
  assert.match(release, /cancel-in-progress: false/);
  assert.match(release, /gh release upload[\s\S]*--clobber/);

  assert.match(trigger, /tags:[\s\S]*- "v\*"/);
  assert.match(trigger, /gh workflow run release\.yml/);
  assert.match(trigger, /-f platform=all/);
  assert.match(
    release,
    /all\) matrix="\[\$macos_arm64,\$macos_x64,\$linux_x64,\$linux_x64_portable,\$linux_arm64,\$linux_arm64_portable,\$windows_x64,\$windows_x86\]"/,
  );
  assert.equal(fs.existsSync(".github/workflows/build-arm-linux.yml"), false);
});

test("Rust workflows share one cache strategy without archiving target", () => {
  const workflows = [
    read(".github/workflows/ci.yml"),
    read(".github/workflows/release.yml"),
  ];

  for (const workflow of workflows) {
    assert.match(workflow, /actions-rust-lang\/setup-rust-toolchain@v1/);
    assert.match(workflow, /cache: false/);
    assert.match(workflow, /mozilla-actions\/sccache-action@v0\.0\.10/);
    assert.match(workflow, /RUSTC_WRAPPER: sccache/);
    assert.match(workflow, /SCCACHE_GHA_ENABLED: "true"/);
    assert.match(
      workflow,
      /key: navop-cargo-inputs-v1-\$\{\{ runner\.os \}\}-\$\{\{ hashFiles\('\*\*\/Cargo\.lock'\) \}\}/,
    );
    assert.doesNotMatch(workflow, /^\s+target\/$/m);
  }

  const ci = workflows[0];
  assert.match(ci, /branches:\s*[\s\S]*?- main/);
  assert.doesNotMatch(ci, /branches:\s*[\s\S]*?- dev/);
  assert.doesNotMatch(ci, /^\s+tags:/m);
  assert.match(ci, /x86_64-unknown-linux-gnu/);
  assert.match(ci, /x86_64-pc-windows-msvc/);
  assert.doesNotMatch(ci, /key: test-cargo-/);

  const release = workflows[1];
  assert.doesNotMatch(release, /key: release-cargo-inputs-/);
  assert.match(release, /actions\/cache\/restore@v4/);
  assert.match(release, /actions\/cache\/save@v4/);
  assert.match(release, /cache-primary-key/);
  assert.match(release, /needs\.prepare\.outputs\.platform != 'all'/);
});

test("application updates prefer navop while accepting legacy package names", () => {
  const install = read("main/src/update/install.rs");

  assert.match(install, /\["navop\.exe", "onetcli\.exe"\]/);
  assert.match(install, /find_file_named\(staging_dir, name\)/);
  assert.match(
    install,
    /\["usr\/bin\/navop", "navop", "usr\/bin\/onetcli", "onetcli"\]/,
  );
});
