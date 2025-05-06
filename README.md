<div align="center">
<img src="https://repository-images.githubusercontent.com/825143915/95735661-0205-4029-97d5-fcfa347c8067" width="45%" height="45%>


#

[![License](https://img.shields.io/github/license/SafaOS/SafaOS?color=red)](https://github.com/SafaOS/SafaOS/blob/main/LICENSE) [![Issues](https://img.shields.io/github/issues/SafaOS/SafaOS)](https://github.com/SafaOS/SafaOS/issues) ![Stars](https://img.shields.io/github/stars/SafaOS/SafaOS?style=flat-square)
</div>

![Screenshot](https://safiworks.github.io/imgs/screenshots/SafaOS-250426.png)

An open-source non-Unix-like OS, written from scratch in Rust for fun.

## Building
the crate at the root of the project is called `safa-helper` it is basically the build system.
you need:

- git
- xorriso
- make
- cargo
- libcurl

first you have to run
```
cargo run init
```
once every rust `libstd` update (or if you are not actively working on the project just once every `git pull` would work).


then to build run
```
cargo run build
```

this should make an iso with the path: `out/safaos.iso` if successful,
you can also find pre-built artifact isos built using github actions, check the latest successful build for the main branch.

## Running
you'll need:

- qemu-system-x86_64

```
cargo run -- --no-kvm
```

or run **with** kvm (faster but kvm might not be available or broken)
```
cargo run
```
otherwise you have the iso `out/safaos.iso` feel free to do whatever you want with it

### Debugging
you can also use the `run.sh` script to debug:
```
cargo run -- --debugger --no-kvm
```
(doesn't work with kvm)
and then connect to port 1234 with a gdb client i recommend using `rust-lldb`.

### Additional Information
```
$ cargo run help
The SafaOS's build system and helper tools

Usage: safa-helper [OPTIONS] [COMMAND]

Commands:
  init   Initializes the submodules and installs the SafaOS's toolchain (rustc target)
  build  Builds a SafaOS iso
  run    Builds and Runs a normal SafaOS iso, requires qemu (default)
  test   Builds and runs a test SafaOS iso, requires qemu
  help   Print this message or the help of the given subcommand(s)

Options:
      --no-kvm           runs with kvm disabled
      --no-gui           runs with gui disabled
      --debugger         runs with debugger enabled on port 1234
  -o, --output <OUTPUT>  The final output of the built iso the default is out/safaos.iso for normal isos and out/safaos-tests.iso for test isos
  -v, --verbose
  -h, --help             Print help
  -V, --version          Print version
```

## Testing
you'll need:

- qemu-system-x86_64

to test SafaOS, run:
```
cargo run test --no-kvm
```
or to test SafaOS alongside the `safa-helper` (currently there are no tests for the helper) run:
```
cargo test
```
the problem is this requires kvm to be enabled, running with `--no-kvm` will not work currently.

## Project structure
`crates-user/`: contains userspace programs written in rust, they are compiled and then copied to the ramdisk as `sys:/bin/`, any rust binary crate added to this directory would be automatically detected, compiled and bundled to the ramdisk by the `safa-helper`.

`crates`: contains the kernel and other kernel-related crates (only the kernel crate is detected by the `safa-helper`).

`ramdisk-include`: anything put in this directory will be included in the ramdisk at `sys:/` by the `safa-helper`.

`common`: some scripts, constants and files that are used by the build system (TODO: move some of the stuff here to ci).

`src`: the source of the SafaOS build system and helper utils aka `safa-helper`.

## Current Features
Check `FEATURES.md`.
you can also check `crates-user/` for examples of programs written in rust.

> Aside from the rust stdandard library another method to interact with the kernel is through the [safa-api](https://github.com/SafaOS/safa-api) which provides low-level wrapper functions around the syscalls, and also some high-level wrappers (such as a userspace allocator which is a very high-level wrapper around the sbrk syscall, ofc the raw sbrk syscall is still exposed).

## Credits
currently uses the [limine](https://limine-bootloader.org/) bootloader.

special thanks to the developers of [MinOS](https://github.com/Dcraftbg/MinOS/), [TacOS](https://github.com/UnmappedStack/TacOS), and [BananaOS](https://github.com/Bananymous/banan-os) for helping develop this (this is my first ever OSDev project)
