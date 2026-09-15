#!/usr/bin/env bash
# Bootstrap the labels used by the AI automation workflow.
# Idempotent: safe to re-run.
#
#   gh auth login
#   bash scripts/ai/create-labels.sh
set -euo pipefail

create() {
  local name="$1" color="$2" desc="$3"
  if gh label create "$name" --color "$color" --description "$desc" >/dev/null 2>&1; then
    echo "created  $name"
  else
    gh label edit "$name" --color "$color" --description "$desc" >/dev/null 2>&1 \
      && echo "updated  $name" \
      || echo "skipped  $name"
  fi
}

create "triage"                     "ededed" "Automated triage completed"
create "triage:bug-ready"           "d73a4a" "Confirmed bug, focused fix"
create "triage:bug-needs-info"      "fbca04" "Bug needs more evidence"
create "triage:feature-quick-win"   "a2eeef" "Small, local feature"
create "triage:feature-defer"       "c5def5" "Too large for automation"
create "triage:already-available"   "0e8a16" "Already shipped"
create "triage:unclear"             "cccccc" "Cannot interpret"
create "triage:other"               "cccccc" "Support or discussion"
create "ready-for-agent"            "5319e7" "Eligible for automated patch"
create "ready-for-human"            "b60205" "Needs a maintainer"
create "needs-info"                 "fbca04" "Waiting on the reporter"
create "invalid-format"             "e4e669" "Failed the issue format gate"
create "automation:bot-pr"          "5319e7" "Opened by automation"
create "automation:review-loop"     "d4c5f9" "Waiting on review findings"
create "automation:review-clean"    "0e8a16" "Review found no issues"
create "format-exempt"              "ffffff" "Skip the issue format gate"
