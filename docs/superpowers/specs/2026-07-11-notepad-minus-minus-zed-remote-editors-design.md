# Notepad-- and Zed Remote Editor Extensions Design

## Goal

Add independently installable static composite extensions that contribute
Notepad-- and Zed as external editors for OnetCli SFTP remote files, then install
the current development build and both extensions on the local macOS machine for
interactive testing.

## Context

OnetCli already supports `contributes.remoteFileEditors`, including platform and
file-mask matching, executable candidates, local executable overrides, temporary
file synchronization, remote conflict detection, and explicit first-launch
confirmation. The extension repository already contains the Windows-only
`notepad-plus-plus-editor` static composite extension. This work reuses that
contract and does not change the OnetCli runtime API.

The local machine has these verified application bundles:

- Notepad-- 3.2.1 at `/Applications/Notepad--.app`, with executable
  `/Applications/Notepad--.app/Contents/MacOS/Notepad--` and bundle identifier
  `www.itdp.cn`.
- Zed 1.9.0 at `/Applications/Zed.app`, with executable
  `/Applications/Zed.app/Contents/MacOS/zed` and bundle identifier
  `dev.zed.Zed`.

## Architecture

Create one extension per editor in the `onetcli-extensions` repository. Each
extension contains only build metadata, `extension.json`, and a short README.
Each is packaged by the existing static composite release path and registered as
an independent marketplace entry.

Keeping the editors separate preserves independent installation, enablement,
versioning, default-editor selection, and executable overrides. It also follows
the existing Notepad++ extension boundary and avoids an editor bundle whose
contents would grow whenever another editor is added.

## Notepad-- Extension

The package id is `notepad-minus-minus-editor`; the runtime extension id is
`com.onetcli.editor.notepad-minus-minus`; and the editor contribution id is
`notepad-minus-minus`.

The contribution:

- is available on macOS;
- matches all files with `*`;
- has priority `100`, making it the preferred editor when both new extensions
  are installed and no explicit default is configured;
- invokes the file directly as the sole `{file}` argument;
- checks `/Applications/Notepad--.app/Contents/MacOS/Notepad--` first;
- supports per-user or other non-standard application locations through
  OnetCli's local executable override. The current candidate expansion contract
  does not expand `${env:HOME}`.

The extension does not bundle, download, or update Notepad--.

## Zed Extension

The package id is `zed-editor`; the runtime extension id is
`com.onetcli.editor.zed`; and the editor contribution id is `zed`.

The extension contains two platform-isolated contributions because the runtime
uses one ordered command candidate list per contribution. Mixing a macOS Bundle
absolute path with a Linux bare command would make fallback selection dependent
on candidate order.

The macOS contribution:

- is available on macOS;
- matches all files with `*`;
- has priority `90`;
- invokes the file directly as the sole `{file}` argument;
- checks `/Applications/Zed.app/Contents/MacOS/zed` on macOS;
- permits non-standard installations through OnetCli's local executable
  override.

The Linux contribution:

- is available on Linux;
- matches all files with `*`;
- has priority `90`;
- invokes the file directly as the sole `{file}` argument;
- uses the bare `zed` command so normal PATH lookup applies;
- has a platform-specific editor id so default-editor and override settings do
  not collide with the macOS contribution.

The extension does not bundle, download, or update Zed.

## Marketplace and Packaging

Each extension receives an `extension.build.json` using `kind: "composite"`,
`language: "static"`, target `universal`, and version `0.1.0`. The marketplace
manifest receives one independent composite entry per package, following the
existing Notepad++ entry format.

The existing release driver must produce:

- `notepad-minus-minus-editor-composite-universal.tar.gz`
- `zed-editor-composite-universal.tar.gz`

Both archives must pass the existing composite package verifier without adding
a WASM module.

## Testing

Extend the extension repository's script tests to assert that both marketplace
entries exist with the expected kind, version, release tag, asset name, and
manifest path. Add fixture-level checks asserting each editor id, platform,
priority, executable candidate list, file mask, and `{file}` argument list.

Run the complete Node script test suite. Then invoke the release driver for both
extensions into a temporary artifact directory and verify both archives through
the normal verifier path.

No OnetCli runtime test changes are required because this design adds data that
uses the already tested contribution contract without changing its behavior.

## Local Installation

Build the current OnetCli `dev` source using the repository's supported macOS
bundle process. Install the resulting application bundle to the established
local application destination without terminating unrelated running processes.

Extract both verified composite archives into the exact configuration directory
scanned by `ExtensionKind::Composite`. The installation must preserve existing
extensions and user configuration. Installation outside the workspace requires
explicit sandbox approval.

## Verification and Acceptance Criteria

The work is accepted when:

1. Both new extension directories contain valid build metadata, runtime
   manifests, and README files.
2. The marketplace manifest registers both extensions independently.
3. The complete extension script test suite passes.
4. Both release-driver builds and composite package verification runs succeed.
5. The locally installed OnetCli application bundle exists and contains the
   newly built executable.
6. Both installed extension manifests exist under the runtime-scanned composite
   extension directory and parse as JSON.
7. OnetCli can discover Notepad-- and Zed on this Mac through their verified
   executable paths.
8. The user can right-click an SFTP remote file, select either editor, confirm
   the first launch, edit and save, and allow OnetCli to synchronize the change
   back through the existing remote-file workflow.

## Non-Goals

- Changing the remote-file editor runtime contract.
- Bundling or installing the third-party editor applications.
- Adding editor-specific process monitoring or save semantics.
- Combining multiple editors into one marketplace extension.
- Changing built-in text editing or image preview behavior.
