#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd -- "$script_dir/../.." && pwd -P)"

if ! command -v vhs >/dev/null 2>&1; then
    echo 'Error: vhs is not on PATH.' >&2
    exit 1
fi

if [[ ! -f "$script_dir/demo.tape" ]]; then
    echo "Error: missing $script_dir/demo.tape" >&2
    exit 1
fi

cd -- "$repo_root"
exec vhs assets/demo/demo.tape
