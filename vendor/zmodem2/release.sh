#!/bin/sh

PREVIOUS="${1:-$PREV}"
NEXT="${2:-$NEXT}"

if [ -z "$NEXT" ] || [ -z "$PREVIOUS" ]; then
  echo "Usage: $0 <PREVIOUS> <NEXT>"
  exit 1
fi

if git status --porcelain | grep .; then
  echo "Working directory $PWD is not clean"
  exit 1
fi

if grep "^version = $NEXT" Cargo.toml >/dev/null; then
  echo "Version $NEXT is already set"
fi

sed -i "s/^version =.*/version = \"$NEXT\"/g" Cargo.toml
cargo clippy --all-targets

git commit -a -s -m "chore: bump version to $NEXT"

MESSAGE=$(
  echo "Release $NEXT"
  echo ""
  git log --pretty=format:"- %s (%an)" --no-merges "$PREVIOUS..HEAD"
)

printf "%s" "$MESSAGE" | git tag -s "$NEXT" -F -
