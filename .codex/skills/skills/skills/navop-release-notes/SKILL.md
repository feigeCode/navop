---
name: navop-release-notes
description: Use when working in the Navop repository on version tags or GitHub Releases, especially when release notes must be generated, reviewed, published, or verified in Chinese and English.
---

# Navop Release Notes

## Overview

Generate Navop GitHub Release notes from git history. Preserve established Release style when it exists; otherwise use the bilingual default in this skill. Use `gh` for GitHub reads and writes, and verify the remote Release after saving.

## Required Context

Work from the Navop repo root and confirm the remote points to `feigeCode/navop`:

```bash
rtk git status --short --branch
rtk git remote -v
rtk git tag --sort=-creatordate
rtk gh release list --limit 8
```

Default version boundary is the newest two version tags, for example `v0.8.8..v0.8.9`. If the user names versions, use those exact tags. Never update a release until the target tag and previous tag are explicit.

Before writing notes, inspect recent Release style:

```bash
rtk gh release view <target-tag> --json tagName,name,body,publishedAt,url
rtk gh release view <previous-tag> --json tagName,name,body,publishedAt,url
rtk gh release view <older-tag> --json tagName,name,body,publishedAt,url
```

If Navop has no established Release style yet, use this bilingual shape:

```markdown
## 中文

### 更新内容

- ...

### 修复与调整

- ...

---

## English

### Changes

- ...

### Fixes

- ...

**Full Changelog**: https://github.com/feigeCode/navop/compare/<previous-tag>...<target-tag>
```

For larger releases, add short overview paragraphs and extra sections only when prior style or commit volume justifies it.

## Generate Notes

Read commits and changed files:

```bash
rtk git log --reverse --format='%H%n%s%n%b%n---END---' <previous-tag>..<target-tag>
rtk git diff --stat <previous-tag>..<target-tag>
rtk git diff --name-status <previous-tag>..<target-tag>
```

For unclear commits, inspect targeted diffs:

```bash
rtk git show --stat --oneline --find-renames <commit>
rtk git show --format=medium --find-renames <commit> -- <path>
```

Summarize user-facing behavior, not implementation trivia. Use categories:

- Chinese `更新内容`: features, UX improvements, performance, workflow improvements.
- Chinese `修复与调整`: bug fixes, compatibility, stability, maintenance with user impact.
- English `Changes`: faithful English version of `更新内容`.
- English `Fixes`: faithful English version of `修复与调整`.

Include maintenance bullets only when visible in commits and useful to release readers. Keep internal refactors out unless they explain a visible behavior change.

## Save Locally

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

## Save To GitHub

Use GitHub CLI release commands, not manual browser editing:

```bash
rtk gh release edit <target-tag> --notes-file /private/tmp/navop-<target-tag>-release-notes.md
```

If the release name is also wrong, set it explicitly:

```bash
rtk gh release edit <target-tag> --title "Navop <target-tag>" --notes-file /private/tmp/navop-<target-tag>-release-notes.md
```

Do not overwrite a non-empty hand-written Release body until you have read it and intentionally preserved or replaced the relevant parts.

## Verify

Before saying the release notes are saved, run a fresh read:

```bash
rtk gh release view <target-tag> --json tagName,name,body,publishedAt,url
```

Verify:

- `tagName` is the intended target tag.
- `body` contains `## 中文`, `## English`, and the expected compare URL.
- The visible content matches the local notes file.

Also check the working tree so unrelated local edits are not mistaken for release-note work:

```bash
rtk git status --short --branch
```

Report the target URL and the verification command result summary.

## Common Mistakes

| Mistake | Correct Action |
| --- | --- |
| Using only Chinese notes | Publish matching Chinese and English sections. |
| Assuming legacy repository details | Confirm the remote is `feigeCode/navop` and use Navop URLs, titles, and temp-file names. |
| Comparing the wrong tags | Confirm newest two tags or use user-specified tags before writing. |
| Listing raw commits | Group commits into release-reader categories. |
| Losing existing release text | Read the existing body first and decide whether to preserve or replace. |
| Claiming saved without verification | Re-read the GitHub Release with `gh release view` first. |
