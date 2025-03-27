#!/bin/bash
git submodule update --init --recursive
cd common
./install-toolchain.sh
