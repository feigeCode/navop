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
    /all\) matrix="\[\$macos_arm64,\$macos_x64,\$linux_x64,\$linux_arm64,\$windows_x64,\$windows_x86\]"/,
  );
  assert.match(release, /linux-x64\) matrix="\[\$linux_x64\]"/);
  assert.match(release, /linux-arm64\) matrix="\[\$linux_arm64\]"/);
  assert.match(release, /name: Build \(\$\{\{ matrix\.target \}\}\)$/m);

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
    /cargo zigbuild[\s\S]*--release[\s\S]*-p main[\s\S]*--target "\$\{\{ matrix\.target \}\}\.2\.28"/,
  );
  // No platform overrides the feature set: `embedded-webview` is off in the
  // default set itself (see main/Cargo.toml), so the shape that gets published is
  // the plain default build on all three platforms. The guard for that lives in
  // its own test below.
  assert.doesNotMatch(build, /--no-default-features/);
  assert.doesNotMatch(build, /--features/);
  assert.match(
    build,
    /cargo build --release -p main --target "\$\{\{ matrix\.target \}\}"/,
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
  assert.match(appPathsSource, /join\(PORTABLE_MARKER_FILE\)\.is_file\(\)/);
  assert.doesNotMatch(appPathsSource, /cfg\(windows\)/);
  assert.doesNotMatch(appPathsSource, /cfg\(target_os = "windows"\)/);
  assert.match(packageLinux, new RegExp(`PORTABLE_MARKER_FILE="${markerFile}"`));
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
    /navop-\*-\$\{\{ matrix\.public_label \}\}-portable\.tar\.gz/,
  );
  // The Windows portable ZIP ships the same contract and hardcodes the same
  // marker name, so both platforms have to agree with the Rust constant.
  assert.match(
    release,
    new RegExp(`"portable-package/${markerFile.replace(/\./g, "\\.")}"`),
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
    /navop-\*-\$\{\{ matrix\.public_label \}\}-gpu-stack\.tar\.gz/,
  );

  assert.match(packageInstallers, /if: matrix\.target == 'x86_64-unknown-linux-gnu'/);
});

test("the embedded webview is an opt-in feature on every platform", () => {
  const workspaceCargo = read("Cargo.toml");
  const cargo = read("crates/ai_chat_view/Cargo.toml");
  const mainCargo = read("main/Cargo.toml");
  const universalPluginsCargo = read("crates/universal-plugins/Cargo.toml");
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

  // The host webview (WebKitGTK 4.1 on Linux) cannot travel with the package, and
  // an in-app HTML preview that works on two platforms but not the third is not
  // worth a second release shape: the feature is off in the default set
  // everywhere and has to be asked for explicitly.
  assert.match(cargo, /^default = \[\]$/m);
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
    /^default = \["wasm-components", "windows-native-rdp", "shell-plugins"\]$/m,
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
    /gpui-component-shell = \{[^}]*optional = true[^}]*\}/,
  );
  assert.match(
    universalPluginsCargo,
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

test("no release build enables the embedded webview, while the opt-in path still compiles", () => {
  const release = read(".github/workflows/release.yml");
  const ci = read(".github/workflows/ci.yml");

  // WebKitGTK 4.1 cannot travel with the package: the distributions that carry it
  // sit on glibc 2.39+, and `WebKitWebProcess` is resolved through a compile-time
  // path. Instead of giving Linux a feature set of its own, the feature is off in
  // the default set (see main/Cargo.toml), so all three platforms publish the same
  // shape and there is no per-platform list to keep in sync.
  assert.doesNotMatch(release, /--features[^\n]*embedded-webview/);
  const build = workflowStep(release, "Build release binary");
  assert.doesNotMatch(build, /--no-default-features/);
  assert.doesNotMatch(build, /--features/);
  assert.match(
    build,
    /cargo zigbuild[\s\S]*--release[\s\S]*-p main[\s\S]*--target "\$\{\{ matrix\.target \}\}\.2\.28"/,
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
  assert.match(optIn, /cargo check -p main --features embedded-webview/);
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
    /all\) matrix="\[\$macos_arm64,\$macos_x64,\$linux_x64,\$linux_arm64,\$windows_x64,\$windows_x86\]"/,
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
  // Every asset, including the Linux GPU dependency stack, is matched by the
  // release globs rather than being enumerated by hand.
  assert.match(release, /navop-\*-\$\{\{ matrix\.public_label \}\}-gpu-stack\.tar\.gz/);
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
    /all\) matrix="\[\$macos_arm64,\$macos_x64,\$linux_x64,\$linux_arm64,\$windows_x64,\$windows_x86\]"/,
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
  const install = read("main/src/update/install.rs");

  assert.match(install, /\["navop\.exe", "onetcli\.exe"\]/);
  assert.match(install, /find_file_named\(staging_dir, name\)/);
  assert.match(
    install,
    /\["usr\/bin\/navop", "navop", "usr\/bin\/onetcli", "onetcli"\]/,
  );
});
