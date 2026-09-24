#!/usr/bin/env bash

set -euo pipefail

script_dir="$(
  cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
  pwd
)"

# The packager needs dataclasses and PEP 585 annotations. RHEL 8 images still
# default to Python 3.6, so pick the newest interpreter that qualifies instead
# of trusting the bare "python3" name.
python=""
for candidate in python3.13 python3.12 python3.11 python3.10 python3.9 python3; do
  command -v "$candidate" >/dev/null 2>&1 || continue
  if "$candidate" -c 'import sys; raise SystemExit(0 if sys.version_info >= (3, 9) else 1)'; then
    python="$candidate"
    break
  fi
done

if [ -z "$python" ]; then
  echo "Error: python 3.9 or newer is required to package the GPU stack" >&2
  exit 1
fi

exec "$python" "$script_dir/package-linux-gpu-stack.py" "$@"
