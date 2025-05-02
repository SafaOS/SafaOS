#!/bin/bash
# A script that build's a SafaOS
# currently there is no arguments, options and etc

# TODO: make the build script more lazy so that it doesn't rebuild everything everytime
set -eo pipefail
echo "Note that ./init.sh must be run at least once before running this script"

ISO_PATH="safaos.iso"
TESTS_ISO_PATH="safaos-tests.iso"
ISO_BUILD_DIR="iso_root"

RUSTC_TOOLCHAIN=$(cd common && ./get-rustc.sh && cd ..)
RAMDISK=()

function build_ramdisk {
    RAMDISK_BUILTIN=()

    cd ramdisk-include
    RAMDISK_INCLUDE=(*)
    cd ..


    # Add all the files in the ramdisk-include directory to the ramdisk root
    for i in "${RAMDISK_INCLUDE[@]}"
    do
	echo "$i: ramdisk-include/$i"
        RAMDISK+=("ramdisk-include/$i" "$i")
    done

    RAMDISK=("${RAMDISK[@]}" "${RAMDISK_BUILTIN[@]}")

    set -- "${RAMDISK[@]}"

    # temporary ramdisk root
    mkdir -pv $ISO_BUILD_DIR/boot/ramdisk

    while [ ! -z  "$1" ] ; do
        O_PATH="$ISO_BUILD_DIR/boot/ramdisk/$2"
        DIR=$(dirname $O_PATH)
        mkdir -pv $DIR
        cp -Trv "$1" $O_PATH
        shift 2
    done

    echo "Creating the init ramdisk"
    tar -cvf $ISO_BUILD_DIR/boot/ramdisk.tar -C $ISO_BUILD_DIR/boot/ramdisk .

    rm -rf $ISO_BUILD_DIR/boot/ramdisk
}

rm -vrf $ISO_BUILD_DIR

if ! (test -d "limine") ; then
    git clone https://github.com/limine-bootloader/limine.git --branch=v8.x-binary --depth=1
fi

make -C limine
mkdir -pv $ISO_BUILD_DIR/boot/limine
mkdir -pv $ISO_BUILD_DIR/EFI/BOOT

function install_toolchain {
    rustup show active-toolchain > /dev/null || rustup toolchain install
    return 0
}

function cargo_build_custom {
    CWD=$(pwd)
    COMMAND=$1
    AT=$2
    ARGS="${@:3}"

    cd "$AT"
    install_toolchain

    cargo $COMMAND $ARGS --message-format=json-render-diagnostics | jq -rs '.[] | select(.reason == "compiler-artifact") | select(.executable != null) | .executable'
}
# TODO: release vs debug mode and such
function cargo_build {
    cargo_build_custom build $@
}

# TODO: release vs debug mode and such
function cargo_build_safaos {
    CWD=$(pwd)
    AT=$1
    ARGS="${@:2}"

    cd "$AT"

    cargo $RUSTC_TOOLCHAIN build $ARGS --target x86_64-unknown-safaos --message-format=json-render-diagnostics | jq -rs '.[] | select(.reason == "compiler-artifact") | select(.executable != null) | .executable'
}

function build_programs {
    SHELL=$(cargo_build_safaos "Shell" --release)
    RAMDISK+=("$SHELL" "bin/safa")

    TESTS=$(cargo_build_safaos "tests" --release)
    RAMDISK+=("$TESTS" "bin/safa-tests")

    BINUTILS=$(cargo_build_safaos "binutils" --release)

    for bin in $BINUTILS;
    do
        base=$(basename $bin)
        RAMDISK+=("$bin" "bin/$base")

    done
}

build_programs
KERNEL_ELF=$(cargo_build "kernel")
KERNEL_TESTS_ELF=$(cargo_build_custom test "kernel" --no-run)

# TODO: make it not build both the tests versions and the normal version
function build_final {
    BOOT=$1
    OUTPUT=$2

    cp -v "$BOOT" $ISO_BUILD_DIR/boot/kernel
    cp -v limine.conf limine/limine-bios.sys limine/limine-bios-cd.bin limine/limine-uefi-cd.bin $ISO_BUILD_DIR/boot/limine
    cp -v limine/BOOTX64.EFI limine/BOOTIA32.EFI $ISO_BUILD_DIR/EFI/BOOT

    build_ramdisk

    echo "Putting the iso together from the iso root directory: $ISO_BUILD_DIR"
    xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
            -no-emul-boot -boot-load-size 4 -boot-info-table \
            --efi-boot boot/limine/limine-uefi-cd.bin \
            -efi-boot-part --efi-boot-image --protective-msdos-label \
            $ISO_BUILD_DIR -o $OUTPUT
}

build_final $KERNEL_ELF $ISO_PATH
build_final $KERNEL_TESTS_ELF $TESTS_ISO_PATH
