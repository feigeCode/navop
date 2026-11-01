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

// 合成一个最小 PE（单 `.rsrc` 节），给 `script/validate-windows-exe.mjs` 做正/负样本。
// 只保证节表能被门禁解析（MZ + PE 签名 + 节表 + 节数据），不追求可执行。
const peFixtureWithResource = (resourcePayload) => {
  const peOffset = 0x40;
  const sectionTable = peOffset + 24; // COFF 头 20 字节 + 可选的 PE32+ 头长度 0
  const resourceOffset = sectionTable + 40;
  const payload = Buffer.from(resourcePayload, "utf8");
  const fixture = Buffer.alloc(resourceOffset + payload.length);

  fixture.write("MZ", 0, "latin1");
  fixture.writeUInt32LE(peOffset, 0x3c);
  fixture.write("PE\0\0", peOffset, "latin1");
  // COFF 头从 PE 签名之后开始（`peOffset + 4`）：Machine / NumberOfSections / …
  fixture.writeUInt16LE(0x8664, peOffset + 4);
  fixture.writeUInt16LE(1, peOffset + 6);
  fixture.writeUInt16LE(0, peOffset + 20);
  fixture.write(".rsrc\0\0\0", sectionTable, "latin1");
  fixture.writeUInt32LE(payload.length, sectionTable + 8);
  fixture.writeUInt32LE(payload.length, sectionTable + 16);
  fixture.writeUInt32LE(resourceOffset, sectionTable + 20);
  payload.copy(fixture, resourceOffset);
  return fixture;
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

test("Linux publishes one package per architecture plus a separate GPU dependency stack", () => {
  const release = read(".github/workflows/release.yml");
  const installZig = workflowStep(release, "Install Zig toolchain (Linux)");
  const build = workflowStep(release, "Build release binary");
  const verifyBaseline = workflowStep(release, "Verify Linux glibc baseline");
  const packageLinux = workflowStep(release, "Package (Linux)");
  const packageGpuStack = workflowStep(
    release,
    "Package Linux GPU dependency stack",
  );
  const packageInstallers = workflowStep(
    release,
    "Package Linux installers (x86_64)",
  );

  // Linux ships exactly one build per architecture. The portable archive is an
  // extra artifact produced from that very same binary inside the same job, not
  // a second matrix entry; the private loader and its launcher are gone, and
  // the Mesa stack travels as a separate dependency archive instead.
  assert.doesNotMatch(release, /portable_linux/);
  assert.doesNotMatch(release, /linux-x64-portable|linux-arm64-portable/);
  assert.doesNotMatch(release, /package-linux-portable|linux-portable-launcher/);
  assert.match(
    release,
    /linux_x64='\{"target":"x86_64-unknown-linux-gnu","os":"ubuntu-latest"[^']*"archive":"navop-x86_64-unknown-linux-gnu\.tar\.gz"[^']*"public_label":"linux-x64"[^']*\}'/,
  );
  assert.match(
    release,
    /linux_arm64='\{"target":"aarch64-unknown-linux-gnu","os":"ubuntu-24\.04-arm"[^']*"archive":"navop-aarch64-unknown-linux-gnu\.tar\.gz"[^']*"public_label":"linux-arm64"[^']*\}'/,
  );
  assert.match(
    release,
    /all\)[\s\S]*?matrix="\[\$\{combined\}\]"/,
  );
  assert.match(release, /linux-x64\)\s*\n\s*full_matrix="\[\$linux_x64\]"\s*\n\s*standalone_matrix="\[\$shell_linux_x64,\$data_linux_x64\]"/);
  assert.match(release, /linux-arm64\)\s*\n\s*full_matrix="\[\$linux_arm64\]"\s*\n\s*standalone_matrix=""/);
  assert.match(release, /name: Build \(\$\{\{ matrix\.product \}\} \/ \$\{\{ matrix\.target \}\}\)/);

  // Zig is what lowers the C runtime requirement to glibc 2.28, so it now runs
  // for every Linux build rather than only for the retired portable variant.
  assert.match(installZig, /if: runner\.os == 'Linux'/);
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
  assert.doesNotMatch(release, /Install portable packaging dependencies/);
  assert.doesNotMatch(release, /musl-tools/);

  assert.match(build, /if \[ "\$\{\{ runner\.os \}\}" = "Linux" \]/);
  assert.match(
    build,
    /cargo zigbuild[\s\S]*--release[\s\S]*-p \$\{\{ matrix\.product \}\}[\s\S]*--target "\$\{\{ matrix\.target \}\}\.2\.28"/,
  );
  // No platform drops the default feature set: `embedded-webview` is off in the
  // default set itself (see crates/navop_app/Cargo.toml), so the shape that gets published is
  // the plain default build on all three platforms. The guard for that lives in
  // its own test below.
  assert.doesNotMatch(build, /--no-default-features/);
  // The Touch Bar workaround feature is switched off again: upstream zed#65186 fixed the
  // root cause (accesskit's plain Adapter instead of the dynamically subclassed content
  // view) and that fix ships in gpui-pre fork-0.3.124, so every platform closes windows by
  // destroying them. If the fix is ever disproven on real hardware, re-adding
  // `--features macos-touchbar-window-hide` for the affected target here is the whole
  // change; the mechanism is still in the code behind `HIDE_WINDOWS_ON_CLOSE`.
  assert.doesNotMatch(
    build,
    /--features/,
    "the build step must not name features: the Intel Mac Touch Bar workaround is disabled now that upstream zed#65186 landed",
  );
  assert.doesNotMatch(build, /extra_features/);
  assert.match(
    build,
    /cargo build --release -p \$\{\{ matrix\.product \}\} --target "\$\{\{ matrix\.target \}\}"/,
  );
  assert.match(
    build,
    /test -x "target\/\$\{\{ matrix\.target \}\}\/release\/\$\{\{ matrix\.binary \}\}"/,
  );

  assert.match(verifyBaseline, /if: runner\.os == 'Linux'/);
  assert.match(
    verifyBaseline,
    /script\/check-linux-glibc-baseline\.sh[\s\S]*target\/\$\{\{ matrix\.target \}\}\/release\/\$\{\{ matrix\.binary \}\}[\s\S]*2\.28/,
  );

  assert.match(packageLinux, /mkdir -p package\/usr\/bin/);
  assert.match(
    packageLinux,
    /cp "target\/\$\{\{ matrix\.target \}\}\/release\/\$\{\{ matrix\.binary \}\}" package\/usr\/bin\//,
  );
  assert.match(packageLinux, /--sort=name/);
  assert.match(packageLinux, /--numeric-owner/);

  // The portable archive carries the identical binary plus the marker file the
  // application looks for next to the executable. It has to be produced from
  // this step without a second compilation, and its marker name must match the
  // constant the Rust side actually reads.
  const appPathsSource = read("crates/core/src/app_paths.rs");
  const markerDeclaration =
    /pub const PORTABLE_MARKER_FILE: &str = "([^"]+)";/.exec(appPathsSource);
  assert.ok(markerDeclaration, "app_paths.rs must declare PORTABLE_MARKER_FILE");
  const markerFile = markerDeclaration[1];
  // Portable mode exists at all only because this detection stays platform
  // independent; a Windows-only guard here would make the Linux archive ship a
  // marker file that nothing ever reads.
  assert.match(appPathsSource, /join\(app_path_names\(\)\.portable_marker\)\s*\.is_file\(\)/);
  assert.doesNotMatch(appPathsSource, /cfg\(windows\)/);
  assert.doesNotMatch(appPathsSource, /cfg\(target_os = "windows"\)/);
  // 统一发布后 marker 名来自 matrix.portable_marker（源头是 identity.rs），
  // 全量条目的值必须仍是 app_paths.rs 读的那个常量。
  assert.match(packageLinux, /PORTABLE_MARKER_FILE="\$\{\{ matrix\.portable_marker \}\}"/);
  assert.match(
    read("crates/domain_api/src/identity.rs"),
    new RegExp(`portable_marker: "${markerFile}"`),
  );
  assert.match(packageLinux, /rm -rf portable-package/);
  assert.match(packageLinux, /mkdir -p portable-package/);
  assert.match(
    packageLinux,
    /cp "target\/\$\{\{ matrix\.target \}\}\/release\/\$\{\{ matrix\.binary \}\}" portable-package\//,
  );
  assert.match(packageLinux, /: > "portable-package\/\$\{PORTABLE_MARKER_FILE\}"/);
  assert.match(
    packageLinux,
    /-czf "\$\{PUBLIC_BASENAME\}-portable\.tar\.gz" \\\n\s+-C portable-package \./,
  );
  // Reusing the release binary is the whole point: no second compilation.
  assert.doesNotMatch(packageLinux, /cargo (?:zig)?build/);
  assert.match(
    release,
    /\$\{\{ matrix\.product \}\}-\*-\$\{\{ matrix\.public_label \}\}-portable\.tar\.gz/,
  );
  // The Windows portable ZIP ships the same contract: the marker name now
  // arrives via matrix.portable_marker, whose full-product value must still
  // equal the Rust constant both platforms read.
  assert.match(
    release,
    /Set-Content -Path "portable-package\/\$\{\{ matrix\.portable_marker \}\}"/,
  );

  // The dependency stack is a second asset for the same target, built against
  // the same glibc 2.28 baseline, and published under the versioned public name.
  assert.match(packageGpuStack, /if: runner\.os == 'Linux'/);
  assert.match(
    packageGpuStack,
    /script\/package-linux-gpu-stack-docker\.sh[\s\S]*--binary "target\/\$\{\{ matrix\.target \}\}\/release\/\$\{\{ matrix\.binary \}\}"[\s\S]*--target "\$\{\{ matrix\.target \}\}"[\s\S]*--output dist-gpu-stack/,
  );
  assert.match(
    packageGpuStack,
    /cp dist-gpu-stack\/navop-gpu-stack-linux-\*\.tar\.gz "\$\{PUBLIC_BASENAME\}-gpu-stack\.tar\.gz"/,
  );
  assert.match(
    release,
    /\$\{\{ matrix\.product \}\}-\*-\$\{\{ matrix\.public_label \}\}-gpu-stack\.tar\.gz/,
  );

  assert.match(packageInstallers, /if: matrix\.product == 'navop' && matrix\.target == 'x86_64-unknown-linux-gnu'/);
});

test("the embedded webview is an opt-in feature on every platform", () => {
  const workspaceCargo = read("Cargo.toml");
  const cargo = read("crates/ai_chat_view/Cargo.toml");
  const mainCargo = read("crates/navop_app/Cargo.toml");
  const universalPluginsCargo = read("crates/universal-plugins/Cargo.toml");
  const htmlCodeBlock = read(
    "crates/ai_chat_view/src/html_code_block.rs",
  );
  const dependentCargoFiles = [
    "crates/navop_app/Cargo.toml",
    "crates/db_view/Cargo.toml",
    "crates/mongodb_view/Cargo.toml",
    "crates/redis_view/Cargo.toml",
    "crates/terminal_view/Cargo.toml",
  ];

  // The host webview (WebKitGTK 4.1 on Linux) cannot travel with the package, and
  // an in-app HTML preview that works on two platforms but not the third is not
  // worth a second release shape: the feature is off in the default set
  // everywhere and has to be asked for explicitly.
  assert.match(cargo, /^default = \[\]$/m);
  assert.match(
    cargo,
    /embedded-webview = \["dep:gpui-webview", "dep:wry"\]/,
  );
  // The dependency key is `gpui-webview`; the workspace manifest pins it to
  // the gpui-kit fork rev that ships the post-rename crate. On GPUI Fast the
  // whole UI stack (webview included) shares the same backend.
  assert.match(cargo, /gpui-webview = \{[^}]*optional = true[^}]*\}/);
  assert.match(
    workspaceCargo,
    /^gpui-webview = \{ git = "https:\/\/github.com\/feigeCode\/gpui-kit",[^}]*\}$/m,
  );
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
    /^default = \["wasm-components", "windows-native-rdp", "shell-plugins", "database", "shell"\]$/m,
  );
  assert.doesNotMatch(mainCargo, /default = \[[^\]]*embedded-webview/);
  assert.match(
    mainCargo,
    /embedded-webview = \["ai_chat_view\/embedded-webview"\]/,
  );
  assert.match(
    mainCargo,
    /shell-plugins = \["universal-plugins\/shell-plugins"\]/,
  );
  assert.match(
    universalPluginsCargo,
    /gpui-shell = \{[^}]*optional = true[^}]*\}/,
  );
  assert.match(
    universalPluginsCargo,
    /shell-plugins = \["dep:gpui-shell"\]/,
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

test("no release build enables the embedded webview, while the opt-in path still compiles", () => {
  const release = read(".github/workflows/release.yml");
  const ci = read(".github/workflows/ci.yml");

  // WebKitGTK 4.1 cannot travel with the package: the distributions that carry it
  // sit on glibc 2.39+, and `WebKitWebProcess` is resolved through a compile-time
  // path. Instead of giving Linux a feature set of its own, the feature is off in
  // the default set (see crates/navop_app/Cargo.toml), so all three platforms publish the same
  // shape and there is no per-platform list to keep in sync.
  assert.doesNotMatch(release, /--features[^\n]*embedded-webview/);
  const build = workflowStep(release, "Build release binary");
  assert.doesNotMatch(build, /--no-default-features/);
  assert.doesNotMatch(build, /--features[^\n]*embedded-webview/);
  assert.doesNotMatch(build, /--features[^\n]*wasm-components/);
  assert.match(
    build,
    /cargo zigbuild[\s\S]*--release[\s\S]*-p \$\{\{ matrix\.product \}\}[\s\S]*--target "\$\{\{ matrix\.target \}\}\.2\.28"/,
  );

  // Windows 32-bit is the one platform that has to name features explicitly (it
  // cannot build shell-plugins). Its list must stay "the default set minus
  // shell-plugins" instead of growing a webview back in.
  assert.match(
    release,
    /--features", "wasm-components,windows-native-rdp"\)/,
  );

  // `cargo test --all` compiles the default set, meaning the half of the code
  // without the feature. CI checks the other half, so the opt-in path keeps
  // compiling instead of rotting until someone asks for it.
  const optIn = workflowStep(ci, "Check the opt-in embedded webview feature");
  assert.match(
    optIn,
    /cargo check -p ai_chat_view --features embedded-webview --tests/,
  );
  assert.match(optIn, /cargo check -p navop_app --features embedded-webview/);
});

test("Linux GPU dependency stack replaces the retired portable runtime", () => {
  const packagerPath = "script/package-linux-gpu-stack.py";
  const wrapperPath = "script/package-linux-gpu-stack.sh";
  const buildPath = "script/package-linux-gpu-stack-build.sh";
  const dockerPath = "script/package-linux-gpu-stack-docker.sh";
  const installPath = "script/linux-gpu-stack-install.sh";

  for (const file of [
    packagerPath,
    wrapperPath,
    buildPath,
    dockerPath,
    installPath,
  ]) {
    assert.ok(fs.existsSync(file), `${file} must exist`);
  }
  // The launcher and the packager that produced it are deliberately gone: the
  // stack now lands on the host loader instead of wrapping the binary.
  for (const retired of [
    "script/package-linux-portable.py",
    "script/package-linux-portable.sh",
    "script/linux-portable-launcher.c",
  ]) {
    assert.equal(fs.existsSync(retired), false, `${retired} must be removed`);
  }

  const wrapper = read(wrapperPath);
  const build = read(buildPath);
  const docker = read(dockerPath);
  const help = spawnSync("python3", [packagerPath, "--help"], {
    encoding: "utf8",
  });

  assert.equal(help.status, 0, help.stderr);
  assert.match(help.stdout, /x86_64-unknown-linux-gnu/);
  assert.match(help.stdout, /aarch64-unknown-linux-gnu/);

  assert.match(wrapper, /set -euo pipefail/);
  assert.match(wrapper, /package-linux-gpu-stack\.py/);
  // RHEL 8 images default to Python 3.6, so the wrapper must not trust it.
  assert.match(wrapper, /sys\.version_info >= \(3, 9\)/);

  // Only a RHEL 8 generation distribution ships a Mesa user space at the
  // glibc 2.28 baseline Navop itself is built against, so dnf is what resolves
  // the tree.
  assert.match(build, /rpm -E %\{rhel\}/);
  assert.match(build, /dnf install -y --setopt=install_weak_deps=False/);
  assert.match(build, /dnf-plugins-core/);
  assert.match(build, /dnf download --resolve/);
  assert.match(
    build,
    /--releasever="\$release_version" --nogpgcheck --forcearch="\$rpm_arch"/,
  );
  assert.match(build, /--installroot "\$empty_root"/);
  assert.match(build, /filter_rpms_to_architecture/);
  assert.match(build, /rpm -qp --qf '%\{ARCH\}'/);
  assert.match(build, /rpm2cpio "\$rpm" \| cpio -idm --no-absolute-filenames/);
  assert.match(build, /mesa-dri-drivers/);
  assert.match(build, /libglvnd-egl/);
  assert.match(build, /libwayland-client/);
  // The bare "liblzma" name does not exist on RHEL 8; the package is xz-libs.
  assert.match(build, /xz-libs/);
  assert.match(
    build,
    /local archive="navop-gpu-stack-linux-\$\{asset_label\}\.tar\.gz"/,
  );
  assert.match(build, /asset_label="x64"/);
  assert.match(build, /asset_label="arm64"/);
  assert.match(build, /--glibc-baseline "\$glibc_baseline"/);
  assert.match(
    build,
    /--installer-source "\$repository_root\/script\/linux-gpu-stack-install\.sh"/,
  );

  // The host side mounts the repository read only plus the binary and the
  // output directory, and must not rely on GNU-only find predicates.
  assert.match(docker, /rockylinux:8/);
  assert.match(docker, /-v "\$repository_root:\/workspace:ro"/);
  assert.match(docker, /-v "\$binary_directory:\/binary:ro"/);
  assert.match(docker, /-v "\$output:\/out"/);
  assert.match(
    docker,
    /bash \/workspace\/script\/package-linux-gpu-stack-build\.sh/,
  );
  assert.match(docker, /navop-gpu-stack-linux-\*\.tar\.gz/);
  assert.doesNotMatch(docker, /-newermt/);
});

test("Linux GPU dependency stack packager separates host libraries from bundled ones", () => {
  const packagerPath = "script/package-linux-gpu-stack.py";
  const python = String.raw`
import importlib.util
import sys

spec = importlib.util.spec_from_file_location("navop_gpu_stack_packager", sys.argv[1])
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)

# Navop reaches EGL through dlopen, so these are roots rather than closure
# members, and Mesa resolves its driver by file name rather than by SONAME.
assert module.GPU_STACK_ENTRY_LIBRARIES == ("libEGL.so.1", "libEGL_mesa.so.0")
assert module.GPU_STACK_DRI_DRIVERS == ("swrast_dri.so", "kms_swrast_dri.so")
assert (
    module.GPU_STACK_EGL_VENDOR_CONFIGURATION
    == "/usr/share/glvnd/egl_vendor.d/50_mesa.json"
)

# The C runtime stays on the host: bundling it would shadow system glibc with a
# foreign copy.
for soname in ("libc.so.6", "libm.so.6", "libpthread.so.0", "ld-linux-x86-64.so.2"):
    assert module.is_host_provided(soname), soname
assert module.is_host_provided("libnss_dns.so.2")
assert not module.is_host_provided("libEGL.so.1")

# The desktop stack integrates with the running session, so it is recorded as a
# host requirement instead of being shipped from one distribution.
for soname in (
    "libgtk-3.so.0",
    "libwebkit2gtk-4.1.so.0",
    "libglib-2.0.so.0",
    "libgobject-2.0.so.0",
    "libpango-1.0.so.0",
    "libcairo.so.2",
):
    assert module.is_host_desktop_library(soname), soname
assert not module.is_host_desktop_library("libEGL.so.1")

# RHEL 8 is merged-/usr: an RPM that owns /lib64/libgcc_s.so.1 has to match the
# extracted /usr/lib64/libgcc_s.so.1, and the other way round.
assert module.path_spellings("/lib64/libgcc_s.so.1") == [
    "/lib64/libgcc_s.so.1",
    "/usr/lib64/libgcc_s.so.1",
]
assert module.path_spellings("/usr/lib64/libfoo.so") == [
    "/lib64/libfoo.so",
    "/usr/lib64/libfoo.so",
]

for name, config in module.TARGET_CONFIGS.items():
    assert config.libdir == "/usr/lib64", name
    assert config.dri_dir == "/usr/lib64/dri", name
    assert config.dri_driver_directories, name
    assert any(entry.endswith("/dri") for entry in config.dri_driver_directories), name
    assert config.architecture_label in ("aarch64", "x86_64"), name
`;
  const result = spawnSync("python3", ["-c", python, packagerPath], {
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
});

test("Linux GPU dependency stack installer is additive and reversibly removable", () => {
  const installPath = "script/linux-gpu-stack-install.sh";
  assert.ok(fs.existsSync(installPath), `${installPath} must exist`);
  const installer = read(installPath);

  assert.match(installer, /set -euo pipefail/);
  assert.match(installer, /--dry-run/);
  assert.match(installer, /--force/);
  assert.match(installer, /--prefix/);
  assert.match(installer, /--uninstall/);

  // Everything lands on paths the loader already searches, so no package form
  // needs environment variables to find it.
  assert.match(installer, /resolve_package_root/);
  assert.match(installer, /load_metadata/);
  assert.match(installer, /verify_host_architecture/);
  assert.match(installer, /ldconfig/);
  assert.match(installer, /soname_is_resolved/);
  assert.match(installer, /host_has_usable_gl/);
  assert.match(installer, /host_has_dri_driver/);
  assert.match(installer, /_dri\.so/);

  // The host's own copy always wins: only gaps are filled.
  assert.match(installer, /already provided by the host/);
  assert.match(installer, /host copy kept/);
  assert.match(installer, /install_gl=0/);

  // Debian-style hosts do not search /usr/lib64, so it is registered, and the
  // registration can be taken back.
  assert.match(installer, /ensure_loader_configuration/);
  assert.match(installer, /navop-gpu-stack\.conf/);
  assert.match(installer, /\/etc\/ld\.so\.conf\.d/);
  assert.match(installer, /drop_loader_configuration/);
  assert.match(installer, /refresh_loader_cache/);

  // Uninstall replays what this installer actually wrote, never the manifest it
  // merely planned: that one also lists the entries the host already provided.
  assert.match(installer, /installed\.tsv/);
  assert.match(installer, /write_install_record/);
  assert.match(installer, /KEPT_FILES\+=/);
  assert.match(installer, /modified since installation/);
  assert.match(installer, /no installation record at/);

  const help = spawnSync("bash", [installPath, "--help"], { encoding: "utf8" });
  assert.equal(help.status, 0, help.stderr);
  assert.match(help.stdout, /--uninstall/);
  // The help body is a sed range over the header comment; the last line must
  // stay inside it.
  assert.match(help.stdout, /\.\/install\.sh --help/);
});

test("Linux install guides document the GPU dependency stack and the portable archive", () => {
  const guides = [
    ["docs-site/docs/guide/install-update.md", /图形依赖包/],
    ["docs-site/docs/en-US/guide/install-update.md", /graphics dependency package/],
    ["docs-site/docs/zh-TW/guide/install-update.md", /圖形相依套件/],
  ];

  for (const [guidePath, heading] of guides) {
    const guide = read(guidePath);
    assert.match(guide, heading, `${guidePath} must document the dependency stack`);
    assert.match(
      guide,
      /navop-<version>-linux-x64-gpu-stack\.tar\.gz/,
      `${guidePath} must name the x86_64 dependency archive`,
    );
    assert.match(
      guide,
      /navop-<version>-linux-arm64-gpu-stack\.tar\.gz/,
      `${guidePath} must name the arm64 dependency archive`,
    );
    // The documented flow has to match the archive layout, the installer name
    // and the flags the script actually accepts.
    assert.match(guide, /tar -xzf navop-<version>-linux-x64-gpu-stack\.tar\.gz -C navop-gpu-stack/);
    assert.match(guide, /sudo navop-gpu-stack\/install\.sh/);
    assert.match(guide, /--dry-run/);
    assert.match(guide, /--force/);
    assert.match(guide, /--uninstall/);
    assert.match(guide, /installed\.tsv/);
    assert.match(guide, /\/usr\/lib\/navop-gpu-stack\//);
    assert.match(guide, /Failed to create surface/);
    // The portable archive ships again, now as the very same binary plus a
    // marker file instead of a private loader, so every guide has to describe
    // the archive names and the marker contract.
    assert.match(
      guide,
      /navop-<version>-linux-x64-portable\.tar\.gz/,
      `${guidePath} must name the x86_64 portable archive`,
    );
    assert.match(
      guide,
      /navop-<version>-linux-arm64-portable\.tar\.gz/,
      `${guidePath} must name the arm64 portable archive`,
    );
    assert.match(
      guide,
      /navop\.portable/,
      `${guidePath} must document the marker file`,
    );
    assert.match(
      guide,
      /--portable\b/,
      `${guidePath} must document the --portable flag`,
    );
    assert.match(
      guide,
      /NAVOP_PORTABLE/,
      `${guidePath} must document the NAVOP_PORTABLE environment variable`,
    );
  }
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
    /"windows-native-rdp"/,
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
    /all\)[\s\S]*?matrix="\[\$\{combined\}\]"/,
  );
  assert.match(
    release,
    /\$\{env:PUBLIC_BASENAME\}-portable\.zip/,
  );
  assert.match(release, /-arch \$\{\{ matrix\.windows_arch \}\}/);
  assert.match(release, /\$\{env:PUBLIC_BASENAME\}\.msi/);
  assert.match(release, /\$\{env:PUBLIC_BASENAME\}\.exe/);
  assert.match(release, /PUBLIC_BASENAME=\$\{\{ matrix\.product \}\}-\$\{VERSION#v\}-\$\{\{ matrix\.public_label \}\}/);

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
    /Set-Content -Path "portable-package\/\$\{\{ matrix\.portable_marker \}\}"/,
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
    /name: \$\{\{ matrix\.product \}\}-\$\{\{ matrix\.public_label \}\}-packages/,
  );
  assert.match(
    release,
    /\$\{\{ matrix\.product \}\}-\*-\$\{\{ matrix\.public_label \}\}-portable\.zip/,
  );
  assert.match(
    release,
    /\$\{\{ matrix\.product \}\}-\*-\$\{\{ matrix\.public_label \}\}\.msi/,
  );
  assert.match(
    release,
    /\$\{\{ matrix\.product \}\}-\*-\$\{\{ matrix\.public_label \}\}\.exe/,
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

test("Windows MSI shortcuts inherit the executable's embedded icon", () => {
  const wix = read("installer/windows/navop.wxs");

  // 快捷方式不能引用 Icon 表：MSI 会把 Icon 表里的图标另存成一份文件，该文件被清理或
  // 图标缓存失效后，快捷方式图标会退化成通用白纸（issue #325）。目标 exe 已内嵌图标。
  for (const id of ["StartMenuShortcut", "DesktopShortcut"]) {
    const shortcut = wix.match(
      new RegExp(`<Shortcut\\b[^>]*Id="${id}"[^>]*/>`),
    );
    assert.ok(shortcut, `missing ${id} shortcut`);
    assert.doesNotMatch(shortcut[0], /\bIcon=/);
  }

  // 控制面板的卸载项仍走 Icon 表；快捷方式图标则由 exe 的内嵌资源提供。
  assert.match(wix, /<Icon Id="NavopIcon" SourceFile="\$\(IconPath\)"\s*\/>/);
  assert.match(wix, /<Property Id="ARPPRODUCTICON" Value="NavopIcon"\s*\/>/);
  assert.match(
    read("crates/navop_app/build.rs"),
    /set_icon\("\.\.\/\.\.\/resources\/windows\/navop\.ico"\)/,
  );
});

test("GitHub and R2 publish every installer while the updater manifest remains compatible", () => {
  const release = read(".github/workflows/release.yml");
  const upload = read(".github/workflows/upload-r2.yml");

  assert.match(
    release,
    /name: \$\{\{ matrix\.product \}\}-\$\{\{ matrix\.public_label \}\}-packages[\s\S]*?\$\{\{ matrix\.product \}\}-\*-\$\{\{ matrix\.public_label \}\}\.msi/,
  );
  assert.match(release, /new_files=\(artifacts\/navop-\* artifacts\/navop_\*\)/);
  // Every asset, including the Linux GPU dependency stack, is matched by the
  // release globs rather than being enumerated by hand.
  assert.match(release, /\$\{\{ matrix\.product \}\}-\*-\$\{\{ matrix\.public_label \}\}-gpu-stack\.tar\.gz/);
  assert.match(upload, /--pattern "navop-\*"/);
  assert.match(upload, /--pattern "navop_\*"/);
  assert.match(upload, /release_files=\(artifacts\/navop-\* artifacts\/navop_\*\)/);
  assert.match(upload, /schema_version: 1/);
  assert.match(upload, /downloads: objectUrls\("releases", product\.updaterAssets\)/);
  assert.match(upload, /fallback_downloads: githubReleaseUrls\(product\.updaterAssets\)/);
  assert.match(upload, /sha256s: objectChecksums\(product\.updaterAssets\)/);
  assert.match(upload, /packages,/);
  assert.match(upload, /publicUpdaterAlternatives/);
  assert.match(upload, /`navop-\$\{version\}-win32\.zip`/);
  assert.match(upload, /\["win32", "i686-pc-windows-msvc"\]/);
  // The portable archives and the dependency stack are extra assets for the
  // same target, so they must map onto that target instead of falling through
  // to "universal", and the distinguishing suffix has to survive in the target
  // name so a download page can tell them apart.
  assert.match(upload, /fileName\.includes\(`-\$\{label\}-gpu-stack\.`\)/);
  assert.match(upload, /fileName\.includes\(`-\$\{label\}-portable\.`\)/);
  assert.match(upload, /return `\$\{publicTarget\[1\]\}-portable`/);
  assert.match(upload, /return `\$\{publicTarget\[1\]\}-gpu-stack`/);
  assert.doesNotMatch(upload, /linux-x64-portable|linux-arm64-portable/);
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

test("R2 keeps only the release it just uploaded", () => {
  const upload = read(".github/workflows/upload-r2.yml");

  // 清理必须排在上传与逐对象校验之后，而且不能总是执行：上传失败时一个对象
  // 都不能删，否则桶里可能新旧两份都不完整。
  assert.ok(
    upload.indexOf("Prune older R2 releases") >
      upload.indexOf("Upload update archives and manifest to R2"),
  );
  const pruneStep = workflowStep(upload, "Prune older R2 releases");
  assert.doesNotMatch(pruneStep, /always\(\)/);
  assert.doesNotMatch(pruneStep, /continue-on-error/);
  assert.match(
    pruneStep,
    /if: \$\{\{ steps\.r2_config\.outputs\.skip != 'true' && steps\.release\.outputs\.skip != 'true' \}\}/,
  );
  // 只删 releases/<tag>/：updates/ 下的清单是更新入口，任何时候都不能删。
  // （prune 会读 updates/beta/latest.json 来保住它指向的安装包，只读不删。）
  assert.match(pruneStep, /prefix="releases"/);
  const pruneDeletes = (pruneStep.match(/aws s3 rm[\s\S]*?--recursive/g) ?? []).join("\n");
  assert.ok(pruneDeletes, "prune must delete through aws s3 rm");
  assert.doesNotMatch(pruneDeletes, /updates\//);
  // 只有稳定版本号才清理，预发布不能把上一个稳定版本的安装包带走。
  assert.match(pruneStep, /=\~ \^v\[0-9\]\+\\\.\[0-9\]\+\\\.\[0-9\]\+\$ \]\]/);
  assert.match(pruneStep, /Skipping R2 prune for non-stable release tag/);

  const pruneRun = pruneStep
    .match(/run: \|\n([\s\S]*)/)?.[1]
    .split("\n")
    .map((line) => line.replace(/^ {10}/, ""))
    .join("\n");
  assert.ok(pruneRun, "prune step must have a run script");

  const fixtureDir = fs.mkdtempSync(path.join(os.tmpdir(), "navop-r2-prune-"));
  const statePath = path.join(fixtureDir, "bucket");
  const deletedPath = path.join(fixtureDir, "deleted");
  const fakeAws = path.join(fixtureDir, "aws");

  // 假 aws 只实现这一步用到的两条命令：列出版本前缀、递归删掉一个前缀。删除
  // 会真的从 fixture 里拿掉版本，所以脚本最后的复查也一起被验证到。
  fs.writeFileSync(
    fakeAws,
    `#!/usr/bin/env bash
set -euo pipefail
case "\${1:-} \${2:-}" in
  "s3api list-objects-v2")
    # 必须带 delimiter，否则列出来的是对象而不是版本目录。
    if [[ " $* " != *" --delimiter / "* ]]; then
      echo "s3api list-objects-v2 without --delimiter: $*" >&2
      exit 42
    fi
    # 跟真实 aws cli 一致：--output text 对扁平列表是「一行、tab 拼接」。
    # 之前这里按一行一个写，恰好把真实现场的解析 bug 掩盖过去了。
    separator=""
    line=""
    for tag in $(grep -v '^$' "\${FAKE_R2_STATE}"); do
      line="\${line}\${separator}releases/\$tag/"
      separator=\$'\t'
    done
    [[ -z "\$line" ]] || printf '%s\\n' "\$line"
    ;;
  "s3api get-object")
    # 只有显式给了 beta 清单才会命中，用于验证「按清单 version 精确保留」。
    if [ -n "\${FAKE_R2_BETA_MANIFEST:-}" ]; then
      for argument in "\$@"; do last="\$argument"; done
      printf '%s' "\${FAKE_R2_BETA_MANIFEST}" > "\$last"
      exit 0
    fi
    exit 1
    ;;
  "s3 rm")
    target=""
    for argument in "\$@"; do
      case "\$argument" in s3://*) target="\$argument" ;; esac
    done
    key="\${target#s3://*/}"
    key="\${key%/}"
    echo "\$key" >> "\${FAKE_R2_DELETED}"
    grep -Fxv "\${key#releases/}" "\${FAKE_R2_STATE}" > "\${FAKE_R2_STATE}.tmp"
    mv "\${FAKE_R2_STATE}.tmp" "\${FAKE_R2_STATE}"
    ;;
  *)
    echo "unexpected aws invocation: \$*" >&2
    exit 42
    ;;
esac
`,
    { mode: 0o755 },
  );

  const runPrune = (tags, releaseTag = "v0.19.5", betaManifest = "") => {
    fs.writeFileSync(statePath, `${tags.join("\n")}\n`);
    fs.writeFileSync(deletedPath, "");
    const result = spawnSync("bash", ["-c", pruneRun], {
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: `${fixtureDir}:${process.env.PATH}`,
        FAKE_R2_STATE: statePath,
        FAKE_R2_DELETED: deletedPath,
        FAKE_R2_BETA_MANIFEST: betaManifest,
        CLOUDFLARE_ACCOUNT_ID: "test-account",
        CLOUDFLARE_R2_BUCKET: "test-bucket",
        RELEASE_TAG: releaseTag,
      },
    });
    return {
      result,
      remaining: fs
        .readFileSync(statePath, "utf8")
        .trim()
        .split("\n")
        .filter(Boolean),
      deleted: fs
        .readFileSync(deletedPath, "utf8")
        .trim()
        .split("\n")
        .filter(Boolean)
        .sort(),
    };
  };

  try {
    const pruned = runPrune(["v0.19.5", "v0.19.4", "v0.19.3"]);
    assert.equal(pruned.result.status, 0, pruned.result.stderr);
    assert.deepEqual(pruned.remaining, ["v0.19.5"]);
    assert.deepEqual(pruned.deleted, [
      "releases/v0.19.3",
      "releases/v0.19.4",
    ]);
    assert.match(pruned.result.stdout, /R2 now keeps .*v0\.19\.5/);

    // 稳定发版不能把 beta 清单指向的安装包删掉，否则 beta 用户点下载会 404。
    // 读不到清单时退回「保留版本号最大的 beta」。
    const betaFallback = runPrune(["v0.19.5", "v0.20.0-beta.2", "v0.19.4"]);
    assert.equal(betaFallback.result.status, 0, betaFallback.result.stderr);
    assert.deepEqual(betaFallback.remaining, ["v0.19.5", "v0.20.0-beta.2"]);
    assert.deepEqual(betaFallback.deleted, ["releases/v0.19.4"]);

    // 能读到清单时必须按清单里的 version 精确保留，而不是笼统留下最大的 beta。
    const betaExact = runPrune(
      ["v0.19.5", "v0.20.0-beta.1", "v0.20.0-beta.3", "v0.19.4"],
      "v0.19.5",
      JSON.stringify({ version: "0.20.0-beta.1" }),
    );
    assert.equal(betaExact.result.status, 0, betaExact.result.stderr);
    assert.deepEqual(betaExact.remaining, ["v0.19.5", "v0.20.0-beta.1"]);
    assert.deepEqual(betaExact.deleted, [
      "releases/v0.19.4",
      "releases/v0.20.0-beta.3",
    ]);

    // 本次 tag 不在桶里（上传其实没成功）时，一个旧版本都不能删。
    const missing = runPrune(["v0.19.4", "v0.19.3"]);
    assert.notEqual(missing.result.status, 0);
    assert.match(missing.result.stdout, /Refusing to prune/);
    assert.deepEqual(missing.deleted, []);
    assert.deepEqual(missing.remaining, ["v0.19.4", "v0.19.3"]);

    // 预发布不清理，既有稳定版本原样保留。
    const prerelease = runPrune(["v0.19.5", "v0.19.4"], "v0.20.0-rc.1");
    assert.equal(prerelease.result.status, 0, prerelease.result.stderr);
    assert.deepEqual(prerelease.deleted, []);
    assert.deepEqual(prerelease.remaining, ["v0.19.5", "v0.19.4"]);
    assert.match(
      prerelease.result.stdout,
      /Skipping R2 prune for non-stable release tag/,
    );
  } finally {
    fs.rmSync(fixtureDir, { recursive: true, force: true });
  }
});

test("release workflow derives the channel and marks only beta releases as prerelease", () => {
  const release = read(".github/workflows/release.yml");
  const upload = read(".github/workflows/upload-r2.yml");

  // prepare job 暴露 channel，后续 step 只从这里取，避免两处各推导一遍。
  assert.match(release, /channel: \$\{\{ steps\.release\.outputs\.channel \}\}/);
  assert.match(release, /echo "channel=\$CHANNEL"/);
  // 与 upload-r2.yml 共用同一条推导规则：纯稳定版本号 stable，其余 beta。
  assert.ok(
    release.includes(
      'if [[ "$RELEASE_TAG" =~ ^v[0-9]+\\.[0-9]+\\.[0-9]+$ ]]; then CHANNEL=stable; else CHANNEL=beta; fi',
    ),
    "release.yml 缺少按 tag 推导通道的逻辑",
  );
  assert.ok(
    upload.includes(
      'if [[ "$release_tag" =~ ^v[0-9]+\\.[0-9]+\\.[0-9]+$ ]]; then CHANNEL=stable; else CHANNEL=beta; fi',
    ),
    "upload-r2.yml 缺少按 tag 推导通道的逻辑",
  );

  const publishStep = workflowStep(release, "Create or update GitHub Release");
  assert.match(publishStep, /CHANNEL: \$\{\{ needs\.prepare\.outputs\.channel \}\}/);
  assert.match(
    publishStep,
    /if \[ "\$CHANNEL" = "beta" \]; then\n\s+prerelease_args\+=\(--prerelease\)/,
  );
  // 稳定通道绝不能出现 --prerelease：误标会让 /releases/latest 跳过稳定版。
  assert.equal(
    (publishStep.match(/--prerelease/g) ?? []).length,
    1,
    "--prerelease 只能出现在 beta 条件分支里",
  );
  // 空数组在 set -u 下的展开必须显式兜底，否则 stable 发布会直接报 unbound variable。
  assert.match(
    publishStep,
    /gh release create "\$RELEASE_TAG"[\s\S]*?\$\{prerelease_args\[@\]\+"\$\{prerelease_args\[@\]\}"\}/,
  );
  assert.match(
    publishStep,
    /gh release edit "\$RELEASE_TAG"[\s\S]*?\$\{prerelease_args\[@\]\+"\$\{prerelease_args\[@\]\}"\}/,
  );
  // dispatch upload-r2 时必须显式带 channel，不能让它自己从 tag 猜。
  assert.match(publishStep, /-f channel="\$CHANNEL"/);

  // upload-r2 的 channel input 与白名单校验。
  assert.match(
    upload,
    /channel:\n\s+description: 发布通道[\s\S]*?required: false\n\s+default: ""\n\s+type: string/,
  );
  assert.match(upload, /CHANNEL="\$\{INPUT_CHANNEL:-\}"/);
  assert.match(upload, /::error::Invalid channel '\$\{CHANNEL\}'; expected stable or beta/);
  // 清单 key 由通道计算，上传行不再硬编码稳定 key。
  assert.match(upload, /stable\)[\s\S]*?manifest_key="updates\/latest\.json"/);
  assert.match(upload, /beta\)[\s\S]*?manifest_key="updates\/beta\/latest\.json"/);
  assert.match(upload, /upload_object "artifacts\/updates\/latest\.json" "\$\{manifest_key\}"/);
  assert.doesNotMatch(
    upload,
    /upload_object "artifacts\/updates\/latest\.json" "updates\/latest\.json"/,
  );
});

test("beta tag uploads the update manifest under updates/beta and never overwrites the stable feed", () => {
  const upload = read(".github/workflows/upload-r2.yml");
  const resolveStep = workflowStep(upload, "Resolve release tag");
  const uploadStep = workflowStep(upload, "Upload update archives and manifest to R2");

  const stepRun = (step) =>
    step
      .match(/run: \|\n([\s\S]*)/)?.[1]
      .split("\n")
      .map((line) => line.replace(/^ {10}/, ""))
      .join("\n");

  const resolveRun = stepRun(resolveStep);
  const uploadRun = stepRun(uploadStep);
  assert.ok(resolveRun, "Resolve release tag step must have a run script");
  assert.ok(uploadRun, "Upload step must have a run script");

  const fixtureDir = fs.mkdtempSync(path.join(os.tmpdir(), "navop-r2-beta-"));
  const workDir = path.join(fixtureDir, "work");
  const objectsPath = path.join(fixtureDir, "objects");
  const outputPath = path.join(fixtureDir, "github-output");

  // 假 gh：本仿真只用到 `gh release view`，Release 存在即成功。
  fs.writeFileSync(
    path.join(fixtureDir, "gh"),
    `#!/usr/bin/env bash
exit 0
`,
    { mode: 0o755 },
  );

  // 假 sha256sum：让仿真在 Linux 与 macOS 上都能跑（macOS 默认没有 sha256sum）。
  fs.writeFileSync(
    path.join(fixtureDir, "sha256sum"),
    `#!/usr/bin/env bash
set -euo pipefail
for file in "$@"; do
  digest="$(node -e 'const crypto=require("crypto"),fs=require("fs");const f=process.argv[1];process.stdout.write(crypto.createHash("sha256").update(fs.readFileSync(f)).digest("hex"))' "$file")"
  printf '%s  %s\\n' "$digest" "$file"
done
`,
    { mode: 0o755 },
  );

  // 假 jq：只覆盖这一步的三种查询（ContentLength / Metadata.sha256 / CacheControl）。
  fs.writeFileSync(
    path.join(fixtureDir, "jq"),
    `#!/usr/bin/env bash
set -euo pipefail
filter="\${!#}"
input="$(cat)"
case "$filter" in
  ".ContentLength")
    sed -n 's/.*"ContentLength":\\([0-9][0-9]*\\).*/\\1/p' <<< "$input"
    ;;
  ".Metadata.sha256 // empty")
    sed -n 's/.*"sha256":"\\([^"]*\\)".*/\\1/p' <<< "$input"
    ;;
  ".CacheControl // empty")
    sed -n 's/.*"CacheControl":"\\([^"]*\\)".*/\\1/p' <<< "$input"
    ;;
  *)
    echo "unexpected jq filter: $filter" >&2
    exit 42
    ;;
esac
`,
    { mode: 0o755 },
  );

  // 假 aws：实现 s3 cp 与 s3api head-object，并把 key 记下来供断言。
  fs.writeFileSync(
    path.join(fixtureDir, "aws"),
    `#!/usr/bin/env bash
set -euo pipefail
case "\${1:-} \${2:-}" in
  "s3 cp")
    shift 2
    source=""
    target=""
    cache_control=""
    metadata=""
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --endpoint-url|--content-type) shift 2 ;;
        --cache-control) cache_control="\${2:-}"; shift 2 ;;
        --metadata) metadata="\${2:-}"; shift 2 ;;
        s3://*) target="$1"; shift ;;
        *) source="$1"; shift ;;
      esac
    done
    key="\${target#s3://*/}"
    printf '%s\\t%s\\t%s\\t%s\\n' "$key" "$source" "$cache_control" "$metadata" >> "\${FAKE_R2_OBJECTS}"
    ;;
  "s3api head-object")
    shift 2
    key=""
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --key) key="\${2:-}"; shift 2 ;;
        --bucket|--endpoint-url|--output) shift 2 ;;
        *) shift ;;
      esac
    done
    row="$(awk -F'\\t' -v key="$key" '$1 == key { print; exit }' "\${FAKE_R2_OBJECTS}")"
    if [ -z "$row" ]; then
      echo "head-object for unknown key: $key" >&2
      exit 1
    fi
    source="$(cut -f2 <<< "$row")"
    cache_control="$(cut -f3 <<< "$row")"
    metadata="$(cut -f4 <<< "$row")"
    size="$(wc -c < "$source" | tr -d '[:space:]')"
    sha="\${metadata#sha256=}"
    printf '{"ContentLength":%s,"Metadata":{"sha256":"%s"},"CacheControl":"%s"}\\n' "$size" "$sha" "$cache_control"
    ;;
  *)
    echo "unexpected aws invocation: $*" >&2
    exit 42
    ;;
esac
`,
    { mode: 0o755 },
  );

  fs.mkdirSync(path.join(workDir, "artifacts", "updates"), { recursive: true });
  fs.writeFileSync(
    path.join(workDir, "artifacts", "navop-x86_64-unknown-linux-gnu.tar.gz"),
    "fake package",
  );
  // 上传段按固定路径读取三份产品清单（由 node 生成步骤产出），仿真里预置。
  for (const productManifest of [
    "updates/latest.json",
    "updates/navop-shell/shell/latest.json",
    "updates/navop-data/data/latest.json",
  ]) {
    const manifestPath = path.join(workDir, "artifacts", productManifest);
    fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
    fs.writeFileSync(manifestPath, '{"schema_version":1}\n');
  }

  const baseEnv = {
    ...process.env,
    PATH: `${fixtureDir}:${process.env.PATH}`,
    GH_TOKEN: "test-token",
    GITHUB_REPOSITORY: "feigeCode/navop",
    FAKE_R2_OBJECTS: objectsPath,
    CLOUDFLARE_ACCOUNT_ID: "test-account",
    CLOUDFLARE_R2_BUCKET: "test-bucket",
  };

  const resolveChannel = (tag, channel) => {
    fs.writeFileSync(outputPath, "");
    const result = spawnSync("bash", ["-c", resolveRun], {
      encoding: "utf8",
      cwd: workDir,
      env: {
        ...baseEnv,
        INPUT_TAG: tag,
        INPUT_CHANNEL: channel,
        GITHUB_OUTPUT: outputPath,
      },
    });
    const outputs = Object.fromEntries(
      fs
        .readFileSync(outputPath, "utf8")
        .split("\n")
        .filter(Boolean)
        .map((line) => {
          const separator = line.indexOf("=");
          return [line.slice(0, separator), line.slice(separator + 1)];
        }),
    );
    return { result, outputs };
  };

  const runUpload = (channel, releaseTag) => {
    fs.writeFileSync(objectsPath, "");
    const result = spawnSync("bash", ["-c", uploadRun], {
      encoding: "utf8",
      cwd: workDir,
      env: { ...baseEnv, RELEASE_TAG: releaseTag, CHANNEL: channel },
    });
    const keys = fs
      .readFileSync(objectsPath, "utf8")
      .split("\n")
      .filter(Boolean)
      .map((row) => row.split("\t")[0]);
    return { result, keys };
  };

  try {
    // beta tag：留空 channel 时按 tag 自动推导，清单只写 beta 前缀。
    const beta = resolveChannel("v0.23.0-beta.1", "");
    assert.equal(beta.result.status, 0, beta.result.stderr);
    assert.equal(beta.outputs.channel, "beta");

    const betaUpload = runUpload("beta", "v0.23.0-beta.1");
    assert.equal(betaUpload.result.status, 0, betaUpload.result.stderr);
    assert.ok(betaUpload.keys.includes("updates/beta/latest.json"));
    assert.ok(
      !betaUpload.keys.includes("updates/latest.json"),
      "beta 发布绝不能写稳定清单 updates/latest.json",
    );
    assert.ok(
      betaUpload.keys.some((key) => key.startsWith("releases/v0.23.0-beta.1/")),
      "beta 安装包仍需按 releases/<tag>/<file> 上传",
    );

    // 稳定 tag：清单仍落在老客户端认识的 updates/latest.json。
    const stable = resolveChannel("v0.23.0", "");
    assert.equal(stable.result.status, 0, stable.result.stderr);
    assert.equal(stable.outputs.channel, "stable");

    const stableUpload = runUpload("stable", "v0.23.0");
    assert.equal(stableUpload.result.status, 0, stableUpload.result.stderr);
    assert.ok(stableUpload.keys.includes("updates/latest.json"));
    // 稳定发布同时把同一份清单写进 beta 键位：beta 客户端只读 beta 清单，
    // 不刷新这里它就永远停在最后一个 beta，收不到新的正式版。
    assert.ok(
      stableUpload.keys.includes("updates/beta/latest.json"),
      "stable 发布必须同时刷新 updates/beta/latest.json",
    );
    assert.ok(stableUpload.keys.includes("updates/navop-shell/shell/latest.json"));
    assert.ok(stableUpload.keys.includes("updates/navop-shell/shell/beta/latest.json"));
    assert.ok(stableUpload.keys.includes("updates/navop-data/data/latest.json"));
    assert.ok(stableUpload.keys.includes("updates/navop-data/data/beta/latest.json"));

    // 显式 channel 优先；非法值必须在推导阶段就被拒绝，不能走到上传。
    assert.equal(resolveChannel("v0.23.0-beta.1", "stable").outputs.channel, "stable");
    const invalid = resolveChannel("v0.23.0", "nightly");
    assert.notEqual(invalid.result.status, 0);
    assert.match(invalid.result.stdout, /::error::Invalid channel/);
  } finally {
    fs.rmSync(fixtureDir, { recursive: true, force: true });
  }
});

test("update feed assets ride the GitHub Release so the CNB mirror can serve them", () => {
  const upload = read(".github/workflows/upload-r2.yml");

  // CNB 只同步 GitHub Release 资产，所以三份清单必须以扁平资产名 feed-*.json
  // 回写上 Release，两种通道各一套命名。
  for (const assetName of [
    "feed-navop.json",
    "feed-navop-beta.json",
    "feed-navop-shell.json",
    "feed-navop-data-beta.json",
  ]) {
    assert.ok(
      upload.includes(assetName),
      `upload-r2.yml 缺少 feed 资产命名 ${assetName}`,
    );
  }

  const publishStep = workflowStep(upload, "Publish update feed assets for mirrors");
  // 同一个 tag 的修复重跑必须能覆盖上一次的 feed 资产。
  assert.match(
    publishStep,
    /gh release upload "\$RELEASE_TAG"[\s\S]*?--repo "\$GITHUB_REPOSITORY"[\s\S]*?--clobber[\s\S]*?artifacts\/feed\/\*\.json/,
  );
  // stable 额外发一份 -beta 资产，内容与稳定清单相同（同一个源文件）：beta
  // 客户端只读 beta 清单/资产，不发这份它永远停在最后一个 beta，收不到新的
  // 正式版。
  assert.match(
    publishStep,
    /if \[ "\$\{CHANNEL\}" = "stable" \]; then[\s\S]*?cp artifacts\/updates\/latest\.json artifacts\/feed\/feed-navop\.json[\s\S]*?cp artifacts\/updates\/latest\.json artifacts\/feed\/feed-navop-beta\.json/,
  );
  // beta 通道只产 beta 命名的资产，稳定资产名归稳定 Release。
  assert.match(
    publishStep,
    /else\n\s+#[^\n]*\n(?:\s+#[^\n]*\n)*\s+cp artifacts\/updates\/latest\.json artifacts\/feed\/feed-navop-beta\.json/,
  );
  assert.match(
    publishStep,
    /else\n[\s\S]*?cp artifacts\/updates\/navop-data\/data\/latest\.json artifacts\/feed\/feed-navop-data-beta\.json/,
  );

  // 必须先按通道写好 R2 清单，再把同一批清单回写成 Release 资产。
  assert.ok(
    upload.indexOf("Publish update feed assets for mirrors") >
      upload.indexOf("Upload update archives and manifest to R2"),
  );

  // feed 资产绝不能被当成安装包：下载过滤与 packages 前缀都只认 navop- / navop_。
  assert.doesNotMatch(upload, /--pattern "feed-\*"/);
  const generateStep = workflowStep(upload, "Generate R2 update manifest");
  assert.match(generateStep, /filter\(\(fileName\) => \/\^navop\[-_\]\/\.test\(fileName\)\)/);
  // 安装包上传循环也只用 navop- / navop_ 通配，feed-*.json 不会进 R2 的
  // releases/<tag>/ 前缀。
  assert.match(upload, /release_files=\(artifacts\/navop-\* artifacts\/navop_\*\)/);
});

test("CNB mirror sync is dispatched after the feed assets exist", () => {
  const upload = read(".github/workflows/upload-r2.yml");
  const release = read(".github/workflows/release.yml");

  const dispatchStep = workflowStep(upload, "Dispatch CNB release asset sync");
  assert.match(
    dispatchStep,
    /gh workflow run sync-cnb-release-assets\.yml[\s\S]*?--repo "\$GITHUB_REPOSITORY"[\s\S]*?--ref main[\s\S]*?-f tag="\$RELEASE_TAG"/,
  );
  // 派发必须在 feed 资产上传之后，而且就是最后一步。
  assert.ok(
    upload.indexOf("Dispatch CNB release asset sync") >
      upload.indexOf("Publish update feed assets for mirrors"),
  );
  const tail = upload.slice(upload.indexOf("Dispatch CNB release asset sync"));
  assert.doesNotMatch(tail, /\n {6}- name:/, "CNB 派发必须是 upload-r2.yml 的最后一步");
  // 它靠 actions: write 才能派发别的 workflow，并靠 contents: write 回写资产。
  assert.match(upload, /permissions:\n {2}actions: write\n {2}contents: write/);

  // release.yml 不再并发派发 CNB 同步：feed 资产还没生成时同步会什么都拿不到。
  assert.doesNotMatch(release, /gh workflow run sync-cnb-release-assets\.yml/);
  // 派发 upload-r2 的那段必须保留，且显式传 channel 与 tag。
  assert.match(
    release,
    /gh workflow run upload-r2\.yml[\s\S]*?--ref main[\s\S]*?-f tag="\$RELEASE_TAG"[\s\S]*?-f channel="\$CHANNEL"/,
  );
});

test("stable releases refresh the beta feed so beta clients can move to the stable build", () => {
  const upload = read(".github/workflows/upload-r2.yml");
  const uploadStep = workflowStep(upload, "Upload update archives and manifest to R2");

  // stable 分支同时写稳定键位与 beta 键位，同一份清单、同样的 no-store 头。
  assert.match(uploadStep, /stable\)\n\s+manifest_key="updates\/latest\.json"/);
  assert.match(
    uploadStep,
    /if \[ "\$\{CHANNEL\}" = "stable" \]; then\n\s+upload_object "artifacts\/updates\/latest\.json" "updates\/beta\/latest\.json" "application\/json; charset=utf-8" "no-store, max-age=0"/,
  );
  assert.match(
    uploadStep,
    /upload_object "artifacts\/updates\/navop-shell\/shell\/latest\.json" "updates\/navop-shell\/shell\/beta\/latest\.json"/,
  );
  assert.match(
    uploadStep,
    /upload_object "artifacts\/updates\/navop-data\/data\/latest\.json" "updates\/navop-data\/data\/beta\/latest\.json"/,
  );
  // 稳定键位不得被硬编码成 beta 键位：上传行仍旧走 ${manifest_key} 变量。
  assert.match(uploadStep, /upload_object "artifacts\/updates\/latest\.json" "\$\{manifest_key\}"/);
  // beta 通道不写稳定键位（仿真用例已覆盖，这里守住脚本形态）。
  assert.doesNotMatch(
    uploadStep,
    /beta\)\n\s+upload_object[\s\S]*?"updates\/latest\.json"/,
  );
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
  assert.match(validator, /Assert-MsiEmptyValue/);
  assert.match(validator, /SELECT Icon_ FROM Shortcut/);
  assert.match(validator, /SELECT KeyPath FROM Component/);
  assert.match(validator, /SELECT Root FROM Registry/);
  assert.match(validator, /\.Trim\(\)/);
  assert.match(validator, /\$null = \$view\.Execute\(\)/);
  assert.match(validator, /\$null = \$view\.Close\(\)/);
  assert.match(validator, /\$value = \[string\]\$record\.StringData\(1\)/);
});

test("release builds use fat LTO with panic=abort and a flat 180min timeout", () => {
  const release = read(".github/workflows/release.yml");
  assert.doesNotMatch(release, /^\s+CARGO_PROFILE_RELEASE_LTO:\s/m);
  assert.doesNotMatch(release, /^\s+CARGO_PROFILE_RELEASE_CODEGEN_UNITS:\s/m);
  assert.match(release, /export CARGO_PROFILE_RELEASE_LTO=thin/);
  assert.match(release, /export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16/);
  assert.match(release, /timeout-minutes: 180/);

  const cargo = read("Cargo.toml");
  assert.match(cargo, /\[profile\.release\][\s\S]*?lto = "fat"/);
  assert.match(cargo, /\[profile\.release\][\s\S]*?codegen-units = 1/);
  assert.match(cargo, /\[profile\.release\][\s\S]*?panic = "abort"/);
});

test("release builds are cacheable and individually repairable", () => {
  const release = read(".github/workflows/release.yml");
  const trigger = read(".github/workflows/release-trigger.yml");

  for (const platform of [
    "macos-arm64",
    "macos-x64",
    "linux-x64",
    "linux-arm64",
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
    /all\)[\s\S]*?matrix="\[\$\{combined\}\]"/,
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
    // sccache is only a build accelerator. A transient GitHub release CDN
    // failure while installing it must degrade to an uncached build instead of
    // failing the job, so the step stays best-effort and unsets the wrapper.
    assert.match(
      workflow,
      /uses: mozilla-actions\/sccache-action@v0\.0\.10\s+continue-on-error: true/,
    );
    assert.match(workflow, /steps\.sccache\.outcome/);
    assert.match(workflow, /echo "RUSTC_WRAPPER=" >> "\$GITHUB_ENV"/);
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
  const install = read("crates/app_kit/src/update/install.rs");

  assert.match(install, /let mut names = vec!\[format!\("\{binary_name\}\.exe"\)/);
  assert.match(install, /find_file_named\(staging_dir, name\)/);
  assert.match(
    install,
    /format!\("usr\/bin\/\{binary_name\}"\)[\s\S]*"usr\/bin\/onetcli"\.to_string\(\),[\s\S]*"onetcli"\.to_string\(\)/,
  );
});

test("Windows 产物由宿主自己内嵌 Common-Controls v6 清单", () => {
  const build = read("crates/navop_app/build.rs");
  const manifest = read("resources/windows/navop.manifest.xml");
  const release = read(".github/workflows/release.yml");

  // 清单内容：gpui 的 Windows 层静态导入 comctl32!TaskDialogIndirect，缺了这条依赖声明
  // 加载器只会给进程挂 System32 里的 comctl32 5.82（没有该导出），navop.exe 会在 main
  // 之前以 0xC0000139 退出。
  assert.match(manifest, /name="Microsoft\.Windows\.Common-Controls"/);
  assert.match(manifest, /version="6\.0\.0\.0"/);
  assert.match(manifest, /publicKeyToken="6595b64144ccf1df"/);
  assert.match(manifest, /level="asInvoker"/);
  assert.match(manifest, /supportedOS/);
  // DPI / 堆设置不能省：gpui 的 Windows 层从不调用 SetProcessDpiAwareness（清单是 DPI 感知的
  // 唯一来源），而 v0.20.0 是从 gpui 自带的清单里拿到 PerMonitorV2 + SegmentHeap 的。这里漏了
  // 会让修复后的进程退回 DPI-unaware —— 高 DPI 下整窗被系统位图拉伸（模糊），
  // 等于修好启动又引入一个新问题。
  assert.match(manifest, /<dpiAware xmlns="[^"]*">true\/pm<\/dpiAware>/);
  assert.match(
    manifest,
    /<dpiAwareness xmlns="[^"]*">PerMonitorV2<\/dpiAwareness>/,
  );
  assert.match(manifest, /<heapType xmlns="[^"]*">SegmentHeap<\/heapType>/);
  // 不能带 XML 声明：winresource 按 RC 字符串逐行写入清单（每行前后各有一个空格），
  // 声明前出现空白不是合法的 XML prolog。上游 Zed 的 gpui.manifest.xml 同样没有声明。
  // （属性用双引号，上游那份用单引号，两者都是合法 XML；本文件的双引号路径是实测过的：
  // winresource 的 escape_string 会把 `"` 写成 `""`，rc 能正确还原。）
  assert.doesNotMatch(manifest, /<\?xml/);

  // 上游 gpui-fast 只在宿主没声明「我自己提供清单」时才嵌（gpui-kit 声明了
  // windows-manifest-provided-by-host，而 Cargo 特性只做并集、下游关不掉），
  // 所以这份清单只能由 navop 自己嵌。
  assert.match(
    build,
    /set_manifest\(include_str!\("\.\.\/\.\.\/resources\/windows\/navop\.manifest\.xml"\)\)/,
  );
  // winresource 只在 VersionInfo::FILETYPE 存在时才写清单块：漏了这行会静默不嵌。
  assert.match(build, /set_version_info\(winresource::VersionInfo::FILETYPE/);
  assert.match(
    build,
    /rerun-if-changed=\.\.\/\.\.\/resources\/windows\/navop\.manifest\.xml/,
  );

  // 发布流程必须在打包前校验真实产物（静态检查拦不住「清单被上游吃掉」这类回归）。
  const validateStep = workflowStep(release, "Validate Windows executable manifest");
  assert.match(validateStep, /node script\/validate-windows-exe\.mjs/);
  assert.match(validateStep, /\$\{\{ matrix\.target \}\}\/release\/\$\{\{ matrix\.binary \}\}/);
  assert.ok(
    release.indexOf("- name: Validate Windows executable manifest") >
      release.indexOf("- name: Build release binary (Windows)"),
    "manifest validation must run after the Windows binary is built",
  );
  assert.ok(
    release.indexOf("- name: Validate Windows executable manifest") <
      release.indexOf("- name: Package (Windows)"),
    "manifest validation must run before the binary is packaged",
  );
});

test("Windows 清单门禁能识别缺失清单的 PE（含真实失败现象的判据）", () => {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "navop-manifest-"));
  const goodExe = path.join(temporary, "good.exe");
  const badExe = path.join(temporary, "bad.exe");
  fs.writeFileSync(
    goodExe,
    peFixtureWithResource(read("resources/windows/navop.manifest.xml")),
  );
  // 真实坏产物的等价物：静态导入了 TaskDialogIndirect，但没有任何清单。
  fs.writeFileSync(badExe, peFixtureWithResource("TaskDialogIndirect"));

  try {
    const ok = spawnSync(
      process.execPath,
      ["script/validate-windows-exe.mjs", goodExe],
      { encoding: "utf8" },
    );
    assert.equal(ok.status, 0, ok.stderr || ok.stdout);

    const bad = spawnSync(
      process.execPath,
      ["script/validate-windows-exe.mjs", badExe],
      { encoding: "utf8" },
    );
    assert.equal(bad.status, 1, bad.stdout);
    assert.match(bad.stderr, /Microsoft\.Windows\.Common-Controls/);
    assert.match(bad.stderr, /TaskDialogIndirect/);
    assert.match(bad.stderr, /0xC0000139/);

    // 非 PE 输入也要报错而不是崩在解析上。
    const notPe = path.join(temporary, "not-pe.exe");
    fs.writeFileSync(notPe, "not a PE file");
    const junk = spawnSync(
      process.execPath,
      ["script/validate-windows-exe.mjs", notPe],
      { encoding: "utf8" },
    );
    assert.equal(junk.status, 1);
    assert.match(junk.stderr, /不是 PE 文件/);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
});

test("release version source stays aligned with the navop product package", () => {
  const versionOf = (source) => source.match(/^version = "([^"]+)"/m)?.[1];
  const mainVersion = versionOf(read("crates/navop_app/Cargo.toml"));
  const navopVersion = versionOf(read("apps/navop/Cargo.toml"));
  assert.ok(mainVersion, "crates/navop_app/Cargo.toml must declare a version");
  assert.equal(navopVersion, mainVersion, "apps/navop/Cargo.toml version must match main");
});

test("standalone products ship their own Linux desktop identity and portable marker", () => {
  const shellDesktop = read("resources/linux/navop-shell.desktop");
  const dataDesktop = read("resources/linux/navop-data.desktop");
  const script = read("script/package-linux-product.sh");
  const shellIdentity = read("crates/domain_api/src/identity.rs");

  assert.match(shellDesktop, /^Name=Navop Shell$/m);
  assert.match(shellDesktop, /^Exec=navop-shell %F$/m);
  assert.match(shellDesktop, /^StartupWMClass=navop-shell$/m);
  assert.match(dataDesktop, /^Name=Navop Data$/m);
  assert.match(dataDesktop, /^Exec=navop-data %F$/m);
  assert.match(dataDesktop, /^StartupWMClass=navop-data$/m);
  assert.match(script, /PORTABLE_MARKER="navop-shell\.portable"/);
  assert.match(script, /PORTABLE_MARKER="navop-data\.portable"/);
  assert.match(shellIdentity, /portable_marker: "navop-shell\.portable"/);
  assert.match(shellIdentity, /portable_marker: "navop-data\.portable"/);
});

test("unified release workflow builds Shell and Data on every primary platform", () => {
  const release = read(".github/workflows/release.yml");

  // 独立产品不再有专属工作流：release.yml 的矩阵直接携带 navop-shell /
  // navop-data 条目，复用与全量一致的构建、打包与上传步骤。
  assert.match(release, /"product":"navop-shell"/);
  assert.match(release, /"product":"navop-data"/);
  assert.match(release, /"archive":"navop-shell-aarch64-apple-darwin\.tar\.gz"/);
  assert.match(release, /"archive":"navop-data-x86_64-pc-windows-msvc\.zip"/);
  assert.match(
    release,
    /script\/bundle-macos\.sh \$\{\{ matrix\.target \}\} \$\{\{ matrix\.product \}\}/,
  );
  assert.match(
    release,
    /script\/package-linux-product\.sh "\$\{\{ matrix\.product \}\}"/,
  );
  assert.match(
    release,
    /cargo build --release -p \$\{\{ matrix\.product \}\} --target "\$\{\{ matrix\.target \}\}"/,
  );
  // 独立产品产物用自身前缀发布，与全量并列。
  assert.match(release, /PUBLIC_BASENAME=\$\{\{ matrix\.product \}\}-\$\{VERSION#v\}-\$\{\{ matrix\.public_label \}\}/);
  assert.equal(
    fs.existsSync(".github/workflows/standalone-products.yml"),
    false,
    "standalone builds belong to the unified release workflow",
  );
});


test("standalone Windows installers keep distinct upgrade identities", () => {
  const script = read("installer/windows/build-product-installer.ps1");
  const msi = read("installer/windows/navop-product.wxs");
  const bundle = read("installer/windows/navop-product-bundle.wxs");
  const full = read("installer/windows/navop.wxs");

  assert.match(script, /"navop-shell"\s*=\s*@\{/);
  assert.match(script, /"navop-data"\s*=\s*@\{/);
  assert.match(script, /6B438332-1757-4910-965B-F85C7B6B0F37/);
  assert.match(script, /E3605D1B-559A-4C46-B03A-A4770236A48A/);
  assert.doesNotMatch(script, /2EAB58E6-27F7-4D97-81A1-7CE4B50FA294/);
  assert.match(msi, /UpgradeCode="\$\(var\.UpgradeCode\)"/);
  assert.match(msi, /Source="\$\(var\.SourceDir\)\\\$\(var\.ExeName\)"/);
  assert.match(bundle, /Id="feigeCode\.\$\(var\.BundleId\)"/);
  assert.doesNotMatch(msi, /CFBundle|ProgId/);
  assert.match(full, /2EAB58E6-27F7-4D97-81A1-7CE4B50FA294/);
});


test("unified release workflow builds Windows installers for both products", () => {
  const release = read(".github/workflows/release.yml");

  // 独立产品在 Windows 上走 build-product-installer.ps1（与原
  // standalone-products.yml 相同的入口），全量仍走 navop.wxs 那套。
  assert.match(
    release,
    /"product":"navop-shell","binary":"navop-shell\.exe","archive":"navop-shell-x86_64-pc-windows-msvc\.zip"/,
  );
  assert.match(
    release,
    /"product":"navop-data","binary":"navop-data\.exe","archive":"navop-data-x86_64-pc-windows-msvc\.zip"/,
  );
  assert.match(release, /build-product-installer\.ps1/);
  assert.match(
    release,
    /-Product "\$\{\{ matrix\.product \}\}"[\s\S]*?-Version "\$env:ONETCLI_VERSION"/,
  );
});
