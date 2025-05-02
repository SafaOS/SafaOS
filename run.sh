#!/bin/bash
# A script to build and run SafaOS using qemu
# arguments:
# no-kvm: runs qemu without kvm
# no-gui: runs qemu withou gui
# debbugger: runs qemu with a gdb server

set -eo pipefail
# Builds an ISO first
./build.sh > last_build.log

KVM=true
GUI=true
TESTS=false
DEBUGGER=false

for arg in "$@"
do
    case $arg in
        "--help")
            echo "Usage: $0 [options]"
            echo "Options:"
            echo "  --no-kvm      Runs qemu without kvm"
            echo "  --no-gui      Runs qemu without gui"
            echo "  --debugger    Runs qemu with a gdb server"
            exit 0
            ;;
        "--no-kvm")
            KVM=false
            ;;
        "--no-gui")
            GUI=false
            ;;
        "--tests")
            TESTS=true
            ;;
        "--debugger")
            DEBUGGER=true
            ;;
        *)
            echo "Unknown argument $arg"
            exit 1
            ;;
    esac
done

QEMU_ARGS=""
if $KVM; then QEMU_ARGS="$QEMU_ARGS -enable-kvm"; fi
if $GUI; then QEMU_ARGS="$QEMU_ARGS -display gtk"; else QEMU_ARGS="$QEMU_ARGS -display none"; fi
if $DEBUGGER; then QEMU_ARGS="$QEMU_ARGS -s -S"; fi

FILE="safaos.iso"
if $TESTS; then FILE="safaos-tests.iso"; fi

QEMU_ARGS="-drive format=raw,file=$FILE -serial stdio -m 512M -bios common/OVMF-pure-efi.fd $QEMU_ARGS"

qemu-system-x86_64 $QEMU_ARGS
