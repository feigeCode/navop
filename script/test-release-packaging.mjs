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
  assert.match(release, /binary: navop\.exe/);
  assert.match(bundle, /BINARY_NAME="navop"/);
  assert.doesNotMatch(bundle, /generate-macos-icon\.sh/);
  assert.match(bundle, /Error: Icon file not found/);
  assert.match(plist, /<key>CFBundleExecutable<\/key>\s*<string>navop<\/string>/);
  assert.match(desktop, /^Exec=navop$/m);
  assert.match(desktop, /^Icon=navop$/m);
  assert.match(desktop, /^StartupWMClass=navop$/m);
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
    /<Shortcut[^>]*Id="DesktopShortcut"[^>]*Directory="DesktopFolder"[^>]*Name="Navop"/,
  );
});

test("GitHub publishes installers while R2 only uploads updater archives", () => {
  const release = read(".github/workflows/release.yml");
  const upload = read(".github/workflows/upload-r2.yml");

  assert.match(release, /artifacts\/navop-\*\.msi/);
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
  assert.match(validator, /\.Trim\(\)/);
  assert.match(validator, /\$null = \$view\.Execute\(\)/);
  assert.match(validator, /\$null = \$view\.Close\(\)/);
  assert.match(validator, /\$value = \[string\]\$record\.StringData\(1\)/);
});

test("release builds use cached-friendly thin LTO", () => {
  for (const workflowPath of [
    ".github/workflows/release.yml",
    ".github/workflows/build-windows-msi.yml",
  ]) {
    const workflow = read(workflowPath);
    assert.match(workflow, /CARGO_PROFILE_RELEASE_LTO: thin/);
    assert.match(workflow, /CARGO_PROFILE_RELEASE_CODEGEN_UNITS: 8/);
  }
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
