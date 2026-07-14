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

test("Windows MSI builds English and Chinese localized installers", () => {
  const release = read(".github/workflows/release.yml");
  const manual = read(".github/workflows/build-windows-msi.yml");
  const wix = read("installer/windows/navop.wxs");
  const englishLocalizationPath = "installer/windows/navop.en-US.wxl";
  const chineseLocalizationPath = "installer/windows/navop.zh-CN.wxl";
  const englishLicensePath = "installer/windows/navop-license-en-US.rtf";
  const chineseLicensePath = "installer/windows/navop-license-zh-CN.rtf";

  assert.match(wix, /Language="\$\(Language\)"/);
  assert.match(wix, /Codepage="\$\(Codepage\)"/);
  assert.match(wix, /WixUILicenseRtf[^]*\$\(LicensePath\)/);
  for (const workflow of [release, manual]) {
    assert.match(workflow, /node script\/generate-windows-license\.mjs/);
    assert.match(workflow, /-culture en-US/);
    assert.match(workflow, /-loc installer\/windows\/navop\.en-US\.wxl/);
    assert.match(workflow, /-culture zh-CN/);
    assert.match(workflow, /-loc installer\/windows\/navop\.zh-CN\.wxl/);
    assert.match(workflow, /-d Language="?1033"?/);
    assert.match(workflow, /-d Language="?2052"?/);
    assert.match(workflow, /navop-x86_64-pc-windows-msvc-zh-CN\.msi/);
  }

  assert.ok(
    fs.existsSync(englishLocalizationPath),
    `${englishLocalizationPath} must exist`,
  );
  assert.ok(
    fs.existsSync(chineseLocalizationPath),
    `${chineseLocalizationPath} must exist`,
  );
  const englishLocalization = read(englishLocalizationPath);
  const chineseLocalization = read(chineseLocalizationPath);
  assert.match(englishLocalization, /Estimated time remaining/);
  assert.match(englishLocalization, /I have read and accept/);
  assert.match(chineseLocalization, /预计剩余时间/);
  assert.match(chineseLocalization, /我已阅读并同意/);

  for (const licensePath of [englishLicensePath, chineseLicensePath]) {
    assert.ok(fs.existsSync(licensePath), `${licensePath} must exist`);
    const license = read(licensePath);
    assert.match(license, /Apache License/);
    assert.match(license, /Navop/);
    assert.doesNotMatch(license, /Lorem ipsum/);
  }
});

test("Windows MSI creates a desktop shortcut", () => {
  const wix = read("installer/windows/navop.wxs");

  assert.match(wix, /<StandardDirectory Id="DesktopFolder"\s*\/>/);
  assert.match(
    wix,
    /<Shortcut[^>]*Id="DesktopShortcut"[^>]*Directory="DesktopFolder"[^>]*Name="Navop"/,
  );
});

test("release publication and R2 upload include the MSI", () => {
  const release = read(".github/workflows/release.yml");
  const upload = read(".github/workflows/upload-r2.yml");

  assert.match(release, /artifacts\/navop-\*\.msi/);
  assert.match(upload, /--pattern "navop-x86_64-pc-windows-msvc\*\.msi"/);
  assert.match(upload, /navop-x86_64-pc-windows-msvc\.msi/);
  assert.match(upload, /navop-x86_64-pc-windows-msvc-zh-CN\.msi/);
});

test("CI runs release packaging regression checks", () => {
  const ci = read(".github/workflows/ci.yml");

  assert.match(ci, /node --test script\/test-release-packaging\.mjs/);
});

test("manual Windows workflow builds a release MSI with its checksum", () => {
  const workflowPath = ".github/workflows/build-windows-msi.yml";
  assert.ok(fs.existsSync(workflowPath), `${workflowPath} must exist`);

  const workflow = read(workflowPath);
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
  assert.match(workflow, /navop-x86_64-pc-windows-msvc\.msi/);
  assert.match(workflow, /navop-x86_64-pc-windows-msvc-zh-CN\.msi/);
  assert.match(workflow, /sha256sums-windows\.txt/);
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
