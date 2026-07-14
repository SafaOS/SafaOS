#!/bin/sh
MANIFEST_PATH=$(pwd)/safa-helper/Cargo.toml
cargo run --manifest-path="$MANIFEST_PATH" -- "$@"
