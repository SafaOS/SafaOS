#!/bin/sh
cd $(pwd)/libkernel
cargo test
cd ..

MANIFEST_PATH=$(pwd)/safa-helper/Cargo.toml
cargo test --manifest-path="$MANIFEST_PATH" -- "$@"
