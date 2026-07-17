#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/target/native-data-drivers}"
PROFILE="${PROFILE:-release}"
TARGET="${TARGET:-}"

cargo_args=(build --profile "$PROFILE" -p onetcli-redis-driver -p onetcli-mongodb-driver)
if [[ -n "$TARGET" ]]; then
  cargo_args+=(--target "$TARGET")
fi

cd "$ROOT"
cargo "${cargo_args[@]}"

target_dir="$ROOT/target"
if [[ -n "$TARGET" ]]; then
  target_dir="$target_dir/$TARGET"
fi
target_dir="$target_dir/$PROFILE"

rm -rf "$OUT"
mkdir -p "$OUT/redis" "$OUT/mongodb-modern" "$OUT/mongodb-legacy"

exe_suffix=""
if [[ "${TARGET:-$(rustc -vV | sed -n 's/^host: //p')}" == *windows* ]]; then
  exe_suffix=".exe"
fi

install -m 755 "$target_dir/onetcli-redis-driver$exe_suffix" \
  "$OUT/redis/onetcli-redis-driver$exe_suffix"
install -m 755 "$target_dir/onetcli-mongodb-modern-driver$exe_suffix" \
  "$OUT/mongodb-modern/onetcli-mongodb-modern-driver$exe_suffix"
install -m 755 "$target_dir/onetcli-mongodb-legacy-driver$exe_suffix" \
  "$OUT/mongodb-legacy/onetcli-mongodb-legacy-driver$exe_suffix"

cp "$ROOT/drivers/redis-driver/driver.json" "$OUT/redis/driver.json"
cp "$ROOT/drivers/mongodb-driver/packages/mongodb-modern/driver.json" \
  "$OUT/mongodb-modern/driver.json"
cp "$ROOT/drivers/mongodb-driver/packages/mongodb-legacy/driver.json" \
  "$OUT/mongodb-legacy/driver.json"

# Keep the manifest self-contained but exclude the checksum file itself: a
# checksum cannot meaningfully validate a file whose contents include its own
# checksum.  Paths are made relative so the list is portable after install.
(
  cd "$OUT"
  find . -type f ! -name SHA256SUMS -print0 \
    | sort -z \
    | xargs -0 shasum -a 256
) > "$OUT/SHA256SUMS"

# Fail packaging if a manifest references a missing executable or if any
# generated artifact does not validate against the checksum list.
while IFS= read -r manifest; do
  entry=$(sed -n 's/.*"command"[[:space:]]*:[[:space:]]*"\(.*\)".*/\1/p' "$manifest" | head -n 1)
  entry=${entry#./}
  test -n "$entry"
  test -x "$(dirname "$manifest")/$entry"
done < <(find "$OUT" -name driver.json -type f | sort)
(cd "$OUT" && shasum -a 256 -c SHA256SUMS)
