#!/bin/sh
# Build the Rust binary from Meson without relying on shell operators being
# passed through Meson's argument vector.
set -eu

cargo="$1"
cargo_home="$2"
manifest="$3"
target_dir="$4"
binary="$5"
output="$6"
shift 6

export CARGO_HOME="$cargo_home"

"$cargo" build \
    --manifest-path "$manifest" \
    --target-dir "$target_dir" \
    "$@"

cp "$binary" "$output"
