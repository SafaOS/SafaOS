<div align="center">
<img src="https://repository-images.githubusercontent.com/825143915/95735661-0205-4029-97d5-fcfa347c8067" width="45%" height="45%>


#

[![License](https://img.shields.io/github/license/SafaOS/SafaOS?color=red)](https://github.com/SafaOS/SafaOS/blob/main/LICENSE) [![Issues](https://img.shields.io/github/issues/SafaOS/SafaOS)](https://github.com/SafaOS/SafaOS/issues) ![Stars](https://img.shields.io/github/stars/SafaOS/SafaOS?style=flat-square)
</div>

![Screenshot](https://safiworks.github.io/priv/imgs/screenshots/SafaOS-270325.png)

An open-source non-Unix-like OS, written from scratch in Rust for fun.

## Building
you need:

- bash
- git
- xorriso
- make
- cargo

simply run
```
./build.sh
```

this should make an iso with the name: `safaos.iso` if successful,
you can also find pre-built artifact isos built using github actions, check the latest successful build for the main branch.

## Running
you'll need:

- qemu-system-x86_64

```
./run.sh
```
or to run without kvm
```
./run.sh no-kvm
```
otherwise you have the iso `safaos.iso` feel free to do whatever you want with it

### Debugging
you can also use the `run.sh` script to debug:
```
./run.sh debugger no-kvm
```
(doesn't work with kvm for now)
and then connect to port 1234 with a gdb client i recommend using `rust-lldb`.

### Additional Information
available arguments for the `run.sh` script are:

- `no-kvm`: disables kvm
- `no-gui`: disables gui
- `debugger`: listens on port 1234 for a debugger

## Testing
there is an automated testing script called `test.sh` which is used to test SafaOS automatically
you'll need:

- qemu-system-x86_64

```
./test.sh
```
the script will return a non-zero exit code if any testing fails

## Current Features
Check `FEATURES.md`.

There is also a bunch of userspace programs written in rust in the `binutils/` directory they are compiled and then copied to the ramdisk as `sys:/bin/`, you can check them out alongside the `tests` for almost everything the kernel is currently capable of.

> Aside from the rust stdandard library another method to interact with the kernel is through the [safa-api](https://github.com/SafaOS/safa-api) which provides low-level wrapper functions around the syscalls, and also some high-level wrappers (such as a userspace allocator which is a very high-level wrapper around the sbrk syscall, ofc the raw sbrk syscall is still exposed).

## Credits
currently uses the [limine](https://limine-bootloader.org/) bootloader.

special thanks to the developers of [MinOS](https://github.com/Dcraftbg/MinOS/), [TacOS](https://github.com/UnmappedStack/TacOS), and [BananaOS](https://github.com/Bananymous/banan-os) for helping develop this (this is my first ever OSDev project)
