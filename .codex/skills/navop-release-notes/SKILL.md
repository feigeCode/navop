---
name: navop-release-notes
description: Use when working in the Navop repository on CHANGELOG.md, version-tag preparation, GitHub Releases, or R2 updater release notes, especially when bilingual Chinese and English notes must be generated, reviewed, synchronized, published, or verified.
---

# Navop Release Notes

## Overview

Maintain Navop's repository-root `CHANGELOG.md` as the single source of truth for user-facing release notes. Generate a bilingual entry before creating the version tag. The Release workflows extract that tagged entry to set both the GitHub Release body and the R2 updater manifest's `release_notes`.

Do not treat GitHub or R2 as the primary editing surface. For a normal release, update and review `CHANGELOG.md`, commit it, and only then create the tag. Only edit an existing GitHub Release directly when the user explicitly asks for a manual synchronization or repair.

## Required Context

Work from the Navop repo root and confirm the remote points to `feigeCode/navop`:

```bash
rtk git status --short --branch
rtk git remote -v
rtk git tag --sort=-creatordate
rtk gh release list --limit 8
```

Confirm all four release inputs:

- Target version tag, for example `v0.10.1`.
- Previous version tag, for example `v0.10.0`.
- Source ref containing the release changes: normally `HEAD` before tagging, or the target tag after it exists.
- ISO release date, for example `2026-08-01`.

If the user names versions, use those exact versions. Otherwise infer the previous tag from the newest stable version tag, but do not invent the target version. Never create a tag, commit, push, or publish unless the user explicitly requested that operation.

Before writing notes, inspect recent Release style. Read the target Release only if it already exists:

```bash
rtk gh release view <previous-tag> --json tagName,name,body,publishedAt,url
rtk gh release view <older-tag> --json tagName,name,body,publishedAt,url
rtk gh release view <target-tag> --json tagName,name,body,publishedAt,url # existing release/repair only
```

If Navop has no established Release style yet, use this bilingual shape:

```markdown
## 中文

### 更新内容

- ...

### 修复与优化

- ...

---

## English

### What's New

- ...

### Fixes and Improvements

- ...

**Full Changelog**: https://github.com/feigeCode/navop/compare/<previous-tag>...<target-tag>
```

For larger releases, add short overview paragraphs and extra sections only when prior style or commit volume justifies it.

## Generate Notes

Read commits and changed files:

```bash
TARGET_REF=HEAD # or the existing target tag
rtk git log --reverse --format='%H%n%s%n%b%n---END---' <previous-tag>.."$TARGET_REF"
rtk git diff --stat <previous-tag>.."$TARGET_REF"
rtk git diff --name-status <previous-tag>.."$TARGET_REF"
```

For unclear commits, inspect targeted diffs:

```bash
rtk git show --stat --oneline --find-renames <commit>
rtk git show --format=medium --find-renames <commit> -- <path>
```

Summarize user-facing behavior, not implementation trivia. Use categories:

- Chinese `更新内容`: features, UX improvements, performance, workflow improvements.
- Chinese `修复与优化`: bug fixes, compatibility, stability, maintenance with user impact.
- English `What's New`: faithful English version of `更新内容`.
- English `Fixes and Improvements`: faithful English version of `修复与优化`.

Include maintenance bullets only when visible in commits and useful to release readers. Keep internal refactors out unless they explain a visible behavior change.

## Prepare The Changelog Entry

Save the generated body to a temporary Markdown file first. Prefer `/private/tmp` on this machine:

```bash
NOTES=/private/tmp/navop-<target-tag>-release-notes.md
```

Create the file with normal editing tools, then review it:

```bash
rtk sed -n '1,240p' /private/tmp/navop-<target-tag>-release-notes.md
```

The file must contain both `## 中文` and `## English`, and the final compare link must use three dots:

```markdown
**Full Changelog**: https://github.com/feigeCode/navop/compare/<previous-tag>...<target-tag>
```

Upsert the reviewed body into `CHANGELOG.md`:

```bash
python3 script/changelog.py upsert \
  --tag <target-tag> \
  --date <YYYY-MM-DD> \
  --notes-file "$NOTES" \
  --changelog CHANGELOG.md
```

The tool inserts the newest version after `<!-- NAVOP_RELEASES -->`, preserves older entries, and replaces an existing target entry idempotently.

Extract it back before approving the entry:

```bash
EXTRACTED=/private/tmp/navop-<target-tag>-release-notes-from-changelog.md
python3 script/changelog.py extract \
  --tag <target-tag> \
  --changelog CHANGELOG.md \
  --output "$EXTRACTED"

diff -u "$NOTES" "$EXTRACTED"
rtk git diff -- CHANGELOG.md
```

The extracted file must match the generated body. Small newline-only differences can be normalized, but content, headings, ordering, and compare URL must be identical.

Commit the reviewed changelog before tagging. `script/release-tag.sh` and `script/bump-version.sh` validate the target entry and fail before tagging if it is missing or malformed.

## Workflow Publishing

For normal releases, do not run `gh release edit` yourself:

1. The Release workflow checks out the requested tag.
2. `script/changelog.py extract` produces the GitHub Release body.
3. The same extracted Markdown is added to R2 `updates/latest.json` as `release_notes`.
4. The application renders either GitHub's Release body or R2's `release_notes`, depending on the selected update source.

Tags older than the changelog infrastructure are legacy-only. Repair workflows may preserve an already-existing GitHub Release body, but they must not create a new changelog-less Release.

## Manual Synchronization

Only when explicitly asked to synchronize an existing GitHub Release, extract from `CHANGELOG.md` first and use that file:

```bash
EXTRACTED=/private/tmp/navop-<target-tag>-release-notes-from-changelog.md
python3 script/changelog.py extract \
  --tag <target-tag> \
  --changelog CHANGELOG.md \
  --output "$EXTRACTED"

rtk gh release edit <target-tag> \
  --title "Navop <target-tag>" \
  --notes-file "$EXTRACTED"
```

Do not compose a different body directly in GitHub. If the desired text changes, update `CHANGELOG.md` first.

## Verify

Always validate the local entry:

```bash
python3 script/changelog.py validate \
  --tag <target-tag> \
  --changelog CHANGELOG.md
```

If a GitHub Release was published or synchronized, run a fresh read:

```bash
rtk gh release view <target-tag> --json tagName,name,body,publishedAt,url
```

Verify:

- `tagName` is the intended target tag.
- `body` contains `## 中文`, `## English`, and the expected compare URL.
- The visible content matches the file extracted from `CHANGELOG.md`.

If R2 was published, read its public updater manifest and verify:

- `version` matches the target tag without the leading `v`.
- `release_notes` contains the same Chinese and English Markdown.
- Download URLs and checksums remain present.

Also check the working tree so unrelated local edits are not mistaken for release-note work:

```bash
rtk git status --short --branch
```

Report the changed changelog entry, whether it was committed/tagged/published, and the verification result. Never claim GitHub or R2 was updated unless a fresh remote read confirms it.

## Common Mistakes

| Mistake | Correct Action |
| --- | --- |
| Using only Chinese notes | Publish matching Chinese and English sections. |
| Creating the tag before the changelog | Generate, review, and commit the target `CHANGELOG.md` entry first. |
| Editing GitHub and changelog separately | Treat `CHANGELOG.md` as authoritative and extract the GitHub body from it. |
| Omitting R2 notes | Confirm `latest.json.release_notes` is populated from the same extracted entry. |
| Assuming legacy repository details | Confirm the remote is `feigeCode/navop` and use Navop URLs, titles, and temp-file names. |
| Comparing the wrong tags | Confirm newest two tags or use user-specified tags before writing. |
| Listing raw commits | Group commits into release-reader categories. |
| Replacing a legacy Release accidentally | Only use the legacy fallback for an already-existing pre-changelog Release. |
| Claiming saved without verification | Validate locally and re-read each remote destination that was actually published. |
