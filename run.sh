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
DEBUGGER=false

for arg in "$@"
do
    case $arg in
        no-kvm)
            KVM=false
            ;;
        no-gui)
            GUI=false
            ;;
        debugger)
            DEBUGGER=true
            ;;
        *)
            echo "Unknown argument $arg"
            exit 1
            ;;
    esac
done

QEMU_ARGS="-drive format=raw,file=safaos.iso -serial stdio -m 512M -bios common/OVMF-pure-efi.fd"
if $KVM; then QEMU_ARGS="$QEMU_ARGS -enable-kvm"; fi
if $GUI; then QEMU_ARGS="$QEMU_ARGS -display gtk"; else QEMU_ARGS="$QEMU_ARGS -display none"; fi
if $DEBUGGER; then QEMU_ARGS="$QEMU_ARGS -s -S"; fi

qemu-system-x86_64 $QEMU_ARGS
