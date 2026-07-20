import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";

const read = (path) => fs.readFileSync(path, "utf8");

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

test("installers register database and Markdown file associations", () => {
  const release = read(".github/workflows/release.yml");
  const plist = read("resources/macos/Info.plist");
  const desktop = read("resources/linux/navop.desktop");
  const wix = read("installer/windows/navop.wxs");
  const mimePath = "resources/linux/navop.xml";

  assert.match(plist, /<key>CFBundleDocumentTypes<\/key>/);
  for (const extension of ["db", "duckdb", "md"]) {
    assert.match(plist, new RegExp(`<string>${extension}<\\/string>`));
    assert.match(wix, new RegExp(`<Extension[^>]*Id="${extension}"`));
  }

  assert.match(
    desktop,
    /^MimeType=.*application\/vnd\.sqlite3;.*application\/x-duckdb;.*text\/markdown;/m,
  );
  assert.ok(fs.existsSync(mimePath), `${mimePath} must exist`);
  const mime = read(mimePath);
  assert.match(mime, /type="application\/vnd\.sqlite3"/);
  assert.match(mime, /pattern="\*\.db"/);
  assert.match(mime, /type="application\/x-duckdb"/);
  assert.match(mime, /pattern="\*\.duckdb"/);
  assert.match(mime, /type="text\/markdown"/);
  assert.match(mime, /pattern="\*\.md"/);
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

test("Windows release builds an installable per-user MSI", () => {
  const release = read(".github/workflows/release.yml");
  const wix = read("installer/windows/navop.wxs");

  assert.match(release, /dotnet tool install --global wix --version 6\.0\.2/);
  assert.match(release, /wix build installer\/windows\/navop\.wxs/);
  assert.match(release, /navop-x86_64-pc-windows-msvc\.msi/);
  assert.match(wix, /Scope="perUser"/);
  assert.match(wix, /StandardDirectory Id="LocalAppDataFolder"/);
  assert.match(wix, /<File[^>]+Source="\$\(SourceDir\)\\navop\.exe"/);
  assert.match(wix, /MajorUpgrade/);
  assert.match(wix, /ProgramMenuFolder/);
  assert.match(wix, /Shortcut[^]*Name="Navop"/);
  assert.match(wix, /RemoveFolder[^]*On="uninstall"/);
  assert.match(wix, /Root="HKCU"/);
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
  const manual = read(".github/workflows/build-windows-msi.yml");
  const wix = read("installer/windows/navop.wxs");
  const localizationPath = "installer/windows/navop.wxl";
  const licensePath = "installer/windows/navop-license.rtf";

  assert.match(wix, /Language="1033"/);
  assert.match(wix, /Codepage="936"/);
  assert.match(wix, /WixUILicenseRtf[^]*navop-license\.rtf/);
  for (const workflow of [release, manual]) {
    assert.match(workflow, /node script\/generate-windows-license\.mjs/);
    assert.match(workflow, /-culture en-US/);
    assert.match(workflow, /-loc installer\/windows\/navop\.wxl/);
    assert.equal(
      (workflow.match(/wix build installer\/windows\/navop\.wxs/g) ?? [])
        .length,
      1,
    );
    assert.doesNotMatch(workflow, /navop-x86_64-pc-windows-msvc-zh-CN\.msi/);
  }

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

test("GitHub publishes installers while R2 only uploads updater archives", () => {
  const release = read(".github/workflows/release.yml");
  const upload = read(".github/workflows/upload-r2.yml");

  assert.match(
    release,
    /name: navop-windows-msi[\s\S]*?path: navop-x86_64-pc-windows-msvc\.msi/,
  );
  assert.match(release, /new_files=\(artifacts\/navop-\* artifacts\/navop_\*\)/);
  assert.match(upload, /navop-x86_64-pc-windows-msvc\.zip/);
  assert.match(upload, /navop-aarch64-apple-darwin\.tar\.gz/);
  assert.match(upload, /navop-x86_64-unknown-linux-gnu\.tar\.gz/);
  assert.doesNotMatch(upload, /\.msi/);
  assert.doesNotMatch(upload, /\.dmg/);
  assert.doesNotMatch(upload, /application\/x-msi/);
  assert.doesNotMatch(upload, /application\/x-apple-diskimage/);

  const uploadList = upload.slice(
    upload.indexOf("release_files=("),
    upload.indexOf('for file in "${release_files[@]}"'),
  );
  assert.doesNotMatch(uploadList, /sha256sums\.txt/);
});

test("CI runs release packaging regression checks", () => {
  const ci = read(".github/workflows/ci.yml");

  assert.match(ci, /node --test script\/test-release-packaging\.mjs/);
  assert.match(ci, /workflow_dispatch:/);
  assert.match(ci, /- windows/);
  assert.match(ci, /fromJSON\(needs\.prepare\.outputs\.matrix\)/);
});

test("manual Windows workflow builds a release MSI with its checksum", () => {
  const workflowPath = ".github/workflows/build-windows-msi.yml";
  const validatorPath = "script/validate-windows-msi.ps1";
  assert.ok(fs.existsSync(workflowPath), `${workflowPath} must exist`);
  assert.ok(fs.existsSync(validatorPath), `${validatorPath} must exist`);

  const workflow = read(workflowPath);
  const validator = read(validatorPath);
  assert.match(workflow, /workflow_dispatch:/);
  assert.match(
    workflow,
    /cargo build --release -p main --target x86_64-pc-windows-msvc/,
  );
  assert.match(workflow, /wix --version 6\.0\.2/);
  assert.match(workflow, /WixToolset\.UI\.wixext\/6\.0\.2/);
  assert.match(workflow, /-ext WixToolset\.UI\.wixext/);
  assert.match(workflow, /Get-FileHash[^]*SHA256/);
  assert.match(workflow, /actions\/upload-artifact@v4/);
  assert.match(workflow, /Compress-Archive[^]*navop-x86_64-pc-windows-msvc\.zip/);
  assert.match(workflow, /Get-FileHash[^]*navop-x86_64-pc-windows-msvc\.zip/);
  assert.match(workflow, /navop-x86_64-pc-windows-msvc\.msi/);
  assert.doesNotMatch(workflow, /navop-x86_64-pc-windows-msvc-zh-CN\.msi/);
  assert.match(workflow, /sha256sums-windows\.txt/);
  assert.match(workflow, /validate-windows-msi\.ps1/);
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

test("release builds keep size-optimized Cargo profile defaults", () => {
  const release = read(".github/workflows/release.yml");
  assert.doesNotMatch(release, /^\s+CARGO_PROFILE_RELEASE_LTO:\s/m);
  assert.doesNotMatch(release, /^\s+CARGO_PROFILE_RELEASE_CODEGEN_UNITS:\s/m);
  assert.match(release, /export CARGO_PROFILE_RELEASE_LTO=thin/);
  assert.match(release, /export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16/);

  const cargo = read("Cargo.toml");
  assert.match(cargo, /\[profile\.release\][\s\S]*?lto = "fat"/);
  assert.match(cargo, /\[profile\.release\][\s\S]*?codegen-units = 1/);

  const manualWindows = read(".github/workflows/build-windows-msi.yml");
  assert.match(manualWindows, /CARGO_PROFILE_RELEASE_LTO: thin/);
  assert.match(manualWindows, /CARGO_PROFILE_RELEASE_CODEGEN_UNITS: 8/);
});

test("release builds are cacheable and individually repairable", () => {
  const release = read(".github/workflows/release.yml");
  const trigger = read(".github/workflows/release-trigger.yml");
  const arm = read(".github/workflows/build-arm-linux.yml");

  for (const platform of [
    "macos-arm64",
    "macos-x64",
    "linux-x64",
    "linux-arm64",
    "windows-x64",
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
  assert.match(arm, /workflows:[\s\S]*- Release/);
  assert.match(arm, /-f platform=linux-arm64/);
});

test("Rust workflows share one cache strategy without archiving target", () => {
  const workflows = [
    read(".github/workflows/ci.yml"),
    read(".github/workflows/release.yml"),
    read(".github/workflows/build-windows-msi.yml"),
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
  assert.match(ci, /branches:\s*[\s\S]*?- dev/);
  assert.match(ci, /x86_64-unknown-linux-gnu/);
  assert.match(ci, /x86_64-pc-windows-msvc/);
  assert.doesNotMatch(ci, /key: test-cargo-/);

  const release = workflows[1];
  assert.doesNotMatch(release, /key: release-cargo-inputs-/);

  const windowsMsi = workflows[2];
  assert.doesNotMatch(windowsMsi, /key: windows-msi-/);
  assert.doesNotMatch(windowsMsi, /github\.run_id/);
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
