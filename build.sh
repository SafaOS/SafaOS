#!/bin/bash
# A script that build's a SafaOS
# currently there is no arguments, options and etc

# TODO: make the build script more lazy so that it doesn't rebuild everything everytime
set -eo pipefail

ISO_PATH="safaos.iso"
ISO_BUILD_DIR="iso_root"

function build_ramdisk {
    RAMDISK_BUILTIN=(
        "bin/zig-out/bin/" "bin/"
        "TestBot/zig-out/bin/TestBot" "bin/TestBot"
        "Shell/zig-out/bin/Shell" "bin/Shell"
    )

    RAMDISK=()
    cd ramdisk-include
    RAMDISK_INCLUDE=(*)
    cd ..

    
    # Add all the files in the ramdisk-include directory to the ramdisk root
    for i in "${RAMDISK_INCLUDE[@]}"
    do
        RAMDISK+=("ramdisk-include/$i" "$i")
    done

    RAMDISK=("${RAMDISK[@]}" "${RAMDISK_BUILTIN[@]}")

    set -- "${RAMDISK[@]}"
    
    # temporary ramdisk root
    mkdir -pv $ISO_BUILD_DIR/boot/ramdisk

    while [ ! -z  "$1" ] ; do
        cp -rv "$1" "$ISO_BUILD_DIR/boot/ramdisk/$2"
        shift 2
    done

    echo "Creating the init ramdisk"
    tar -cvf $ISO_BUILD_DIR/boot/ramdisk.tar -C $ISO_BUILD_DIR/boot/ramdisk .

    rm -rf $ISO_BUILD_DIR/boot/ramdisk
}

rm -vrf $ISO_BUILD_DIR
git submodule update --init --recursive

if ! (test -d "limine") ; then
    git clone https://github.com/limine-bootloader/limine.git --branch=v8.x-binary --depth=1
fi

make -C limine
mkdir -pv $ISO_BUILD_DIR/boot/limine
mkdir -pv $ISO_BUILD_DIR/EFI/BOOT

function cargo_build {
    CWD=$(pwd)
    AT=$1
    ARGS="${@:2}"

    cd "$AT"
    json=$(cargo build $ARGS --message-format=json-render-diagnostics)
    printf "%s" "$json" | jq -js '[.[] | select(.reason == "compiler-artifact") | select(.executable != null)] | last | .executable'
}

function zig_build {
    CWD=$(pwd)
    AT=$1
    ARGS="${@:2}"

    cd "$AT"
    zig build $ARGS
    cd "$CWD"
}

function build_programs {
    zig_build "Shell"
    zig_build "TestBot"
    zig_build "bin"
}

build_programs
KERNEL_ELF=$(cargo_build "kernel" --features=test)

cp -v "$KERNEL_ELF" $ISO_BUILD_DIR/boot/kernel
cp -v limine.conf limine/limine-bios.sys limine/limine-bios-cd.bin limine/limine-uefi-cd.bin $ISO_BUILD_DIR/boot/limine
cp -v limine/BOOTX64.EFI limine/BOOTIA32.EFI $ISO_BUILD_DIR/EFI/BOOT

build_ramdisk

echo "Putting the iso toghether from the iso root directory: $ISO_BUILD_DIR"
xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
            -no-emul-boot -boot-load-size 4 -boot-info-table \
            --efi-boot boot/limine/limine-uefi-cd.bin \
            -efi-boot-part --efi-boot-image --protective-msdos-label \
            $ISO_BUILD_DIR -o $ISO_PATH
