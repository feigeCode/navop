#!/bin/bash

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

# Check if version argument is provided
new_version=$1
if [ -z "$new_version" ]
then
  echo -e "${RED}${BOLD}Error:${RESET} Version argument is required"
  echo -e "${YELLOW}USAGE:${RESET} ./bump.sh [VERSION]"
  exit 1
fi

# Logging functions
function log_header() {
  local message=$1
  echo ""
  echo -e "${BOLD}${BLUE}╔════════════════════════════════════════════════════════╗${RESET}"
  echo -e "${BOLD}${BLUE}║${RESET}  ${CYAN}${BOLD}$message${RESET}"
  echo -e "${BOLD}${BLUE}╚════════════════════════════════════════════════════════╝${RESET}"
  echo ""
}

function log_step() {
  local step=$1
  local message=$2
  echo -e "${MAGENTA}${BOLD}[$step]${RESET} ${message}"
}

function log_success() {
  local message=$1
  echo -e "${GREEN}${BOLD}✓${RESET} ${message}"
}

function log_info() {
  local message=$1
  echo -e "${CYAN}ℹ${RESET} ${message}"
}

function log_error() {
  local message=$1
  echo -e "${RED}${BOLD}✗${RESET} ${message}"
}

# Start release process
log_header "Starting Release Process for v$new_version"

# Step 1: Validate changelog
log_step "1/5" "Validating bilingual CHANGELOG.md entry for ${BOLD}v$new_version${RESET}"
if python3 script/changelog.py validate --tag "v$new_version" --changelog CHANGELOG.md; then
  log_success "Changelog entry is ready"
else
  log_error "Missing or invalid changelog entry; generate it before tagging"
  exit 1
fi
if [ -n "$(git status --porcelain)" ]; then
  log_error "Working tree is not clean"
  log_info "Generate, review, and commit CHANGELOG.md before running this release script"
  exit 1
fi
if ! git ls-files --error-unmatch CHANGELOG.md script/changelog.py >/dev/null 2>&1; then
  log_error "CHANGELOG.md and script/changelog.py must be tracked by Git"
  exit 1
fi
echo ""

# Step 2: Update crates version
log_step "2/5" "Updating crates to version ${BOLD}v$new_version${RESET}"
if cargo set-version "$new_version"; then
  log_success "Crates version updated successfully"
else
  log_error "Failed to update crates version"
  exit 1
fi
echo ""

# Step 3: Stage changes
log_step "3/5" "Staging modified files"
if git add -u .; then
  log_success "Files staged successfully"
else
  log_error "Failed to stage files"
  exit 1
fi
echo ""

# Step 4: Create commit and tag
log_step "4/5" "Creating commit and tag"
if git commit -m "Bump v$new_version"; then
  log_success "Commit created: ${BOLD}Bump v$new_version${RESET}"
else
  log_error "Failed to create commit"
  exit 1
fi

if git tag "v$new_version"; then
  log_success "Tag created: ${BOLD}v$new_version${RESET}"
else
  log_error "Failed to create tag"
  exit 1
fi
echo ""

# Step 5: Push to remote
log_step "5/5" "Pushing tag to remote"
log_info "Pushing ${BOLD}v$new_version${RESET} to origin..."
if git push origin "v$new_version"; then
  log_success "Tag pushed to remote successfully"
else
  log_error "Failed to push tag to remote"
  exit 1
fi
echo ""

# Success message
echo -e "${GREEN}${BOLD}╔════════════════════════════════════════════════════════╗${RESET}"
echo -e "${GREEN}${BOLD}║${RESET}  ${BOLD}🚀 Release v$new_version standby!${RESET}"
echo -e "${GREEN}${BOLD}║${RESET}  ${GREEN}Let's ship it!${RESET}"
echo -e "${GREEN}${BOLD}╚════════════════════════════════════════════════════════╝${RESET}"
echo ""
