# Source Discovery

Use this to find where a desktop application stores connection profiles and how to declare those files in an importer manifest.

## Platform Search Areas

Common macOS paths:

```text
~/Library/Application Support/<Vendor>/<Product>/
~/Library/Application Support/<Product>/
~/Library/Preferences/
~/Library/Containers/<bundle-id>/Data/Library/Application Support/
~/Library/Group Containers/<group-id>/
```

Common Windows paths:

```text
%APPDATA%/<Vendor>/<Product>/
%LOCALAPPDATA%/<Vendor>/<Product>/
%USERPROFILE%/.<tool>/
%USERPROFILE%/Documents/<Product>/
```

Common Linux paths:

```text
~/.config/<product>/
~/.local/share/<product>/
~/.<product>/
```

Product editions often change paths. Look for variants such as Lite, Premium, Community, Pro, Store, portable, cloud workspace, team workspace, and beta channel.

## Discovery Commands

Use targeted searches, not broad home-directory scans:

```bash
rtk find ~/Library/Application\ Support -maxdepth 4 -iname '*navicat*'
rtk find ~/.config ~/.local/share -maxdepth 4 -iname '*dbeaver*'
rtk find ~/.ssh -maxdepth 2 -type f
```

For Windows paths, inspect documentation, issue reports, or user-provided paths, then encode `%APPDATA%`, `%LOCALAPPDATA%`, or `%USERPROFILE%` in `extension.json` instead of absolute user-specific paths.

Useful format probes:

```bash
rtk file <path>
rtk plutil -p <path>
rtk sqlite3 <path> '.tables'
rtk head -40 <path>
```

Do not paste secrets into logs or final answers. When a fixture is needed, scrub values while preserving keys, nesting, encodings, and representative edge cases.

## Candidate File Rules

Each candidate file needs:

- A stable `id`.
- A platform if the path is platform-specific.
- A path using supported expansion syntax such as `~/...`, `%APPDATA%/...`, `%LOCALAPPDATA%/...`, or `%USERPROFILE%/...`.
- A matching `fs:read:<path>` permission in the same manifest.

Prefer multiple explicit candidates over one vague path when editions differ. It makes scan reports, logs, and test failures easier to diagnose.

## Directory Candidates

Use directory reads when the application stores one profile per child file or folder. The host WIT supports:

- `read-directory(candidate-id)`
- `read-candidate-child-file(candidate-id, relative-path)`

Guard directory traversal:

- Only read expected file names or extensions.
- Ignore nested paths unless the product format requires them.
- Keep relative paths normalized.

## Scan Semantics

`scan` should answer source availability, not import every record:

- `available`: at least one candidate exists and can plausibly be parsed.
- `no-data`: candidates were checked but no data files exist.
- `permission-required`: host denied access.
- `error`: a real unexpected failure occurred.

Include `discovered_files` so the UI and logs show which source was found. If estimated record count is expensive, leave `estimated_count` empty.

## Discovery Checklist

- Check every product edition named in the user request.
- Check platform analogues for every macOS path added to the manifest.
- Verify whether config format can be binary as well as text.
- Verify whether type information lives in a field, filename, parent folder, or profile id.
- Identify parameter blocks that are not connection records.
- Capture at least one fixture for each schema variant.
