#!/bin/sh
cd $(pwd)/libakernel
cargo test
cd ..

MANIFEST_PATH=$(pwd)/safa-helper/Cargo.toml
cargo test --manifest-path="$MANIFEST_PATH" -- "$@"
