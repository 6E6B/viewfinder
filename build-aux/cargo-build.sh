#!/usr/bin/env bash
set -euo pipefail
cmd=("$1" build --locked --release --manifest-path "$2/Cargo.toml" --target-dir "$3")
case "${CARGO_NET_OFFLINE:-true}" in
  0|false|FALSE|no|NO) ;;
  *) cmd+=(--offline) ;;
esac
"${cmd[@]}"
install -m 755 "$3/release/viewfinder" "$4"
