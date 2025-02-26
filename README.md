<div align="center">
<img src="https://repository-images.githubusercontent.com/825143915/95735661-0205-4029-97d5-fcfa347c8067" width="45%" height="45%>

#
[![License](https://img.shields.io/github/license/SafaOS/SafaOS?color=red)](https://github.com/SafaOS/SafaOS/blob/main/LICENSE) [![Issues](https://img.shields.io/github/issues/SafaOS/SafaOS)](https://github.com/SafaOS/SafaOS/issues) ![Stars](https://img.shields.io/github/stars/SafaOS/SafaOS?style=flat-square)
</div>

![Screenshot](https://observerunit.github.io/priv/imgs/screenshots/Safa190225.png)

An open-source non-Unix-like open-source OS, written from scratch in Zig and Rust for fun,
the main language is Rust, Zig is used for lower-level userspace stuff (not used in the kernel at all) for now, as an alternative to C
**star the repo!**

## Building
you need: 

- bash
- git
- xorriso
- make
- cargo
- zig

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
avalable arguments for the `run.sh` script are:

- `no-kvm`: disables kvm
- `no-gui`: disables gui
- `debugger`: listens on port 1234 for a debugger

## Testing
there is an automated testing script called `test.sh` which is used to test SafaOS automatcally
you'll need:

- qemu-system-x86_64

```
./test.sh
```
the script will return a non-zero exit code if any testing fails

## Current Features
there is a bunch of userspace programs written in zig in the `bin/` directory they are compiled with zig and then copied to the ramdisk as `sys:/bin/`, you can check them out for almost everything the OS is currently capable of, (also checkout the `Shell/`)

## Credits
currently uses [limine](https://limine-bootloader.org/) bootloader
special thanks to the developers of [MinOS](https://github.com/Dcraftbg/MinOS/), [TacOS](https://github.com/UnmappedStack/TacOS), and [BananaOS](https://github.com/Bananymous/banan-os) for helping develop this (this is my first ever OSDev project)
