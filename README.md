<div align="center">

[![License](https://img.shields.io/github/license/SafaOS/SafaOS?color=red)](https://github.com/SafaOS/SafaOS/blob/main/LICENSE) [![Issues](https://img.shields.io/github/issues/SafaOS/SafaOS)](https://github.com/SafaOS/SafaOS/issues) ![Stars](https://img.shields.io/github/stars/SafaOS/SafaOS?style=flat-square)


</div>

# SafaOS
badly written open-source generic operating system made for fun written in rust!
i am attempting to make something like ChromeOS with native wasm support
this is my first OS!
**previously known as NaviOS**

**this project is written in rust and zig** which is inconvenience and expensive i know, but this was made for fun and learning purposes, even so our primary goal is the runtime results.
**star the repo!**

## building
you need: 

- bash
- git
- xorriso
- make
- cargo
- zig

simply run
```
cargo build
```

this should make an iso with the name: `safaos.iso` if successful
## running with OSHelper
the main crate called `SafaOS` (let's call it OsHelper), which is a simple wrapper around qemu-system-x86_64 to run the iso

you'll need:

- qemu-system-x86_64

and simply do
```
cargo run
```
or to run without kvm do
```
cargo run -- no-kvm
```
otherwise you have the iso feel free to do whatever you want with it

### debugging
you can also use the OsHelper to debug, simply do
```
cargo run -- debugger no-kvm
```
(doesn't work with kvm)
and then connect to port 1234 with a gdb client i recommend using `rust-lldb`

### additional information
avalable arguments for the OsHelper are:

- `no-kvm`: disables kvm
- `no-gui`: disables gui
- `debugger`: listens on port 1234 for a debugger

## testing
there is an automated testing script called `test.sh` which is used to test SafaOS automatcally
you need:

- qemu-system-x86_64

simply run
```
./test.sh
```
the script will return a non-zero exit code if any testing fails

## current features:
there is a bunch of userspace programs written in zig in the `bin/` directory they are compiled with zig and then copied to the ramdisk as `sys:/bin/`, you can check them out for almost everything the OS is currently capable of, (also checkout the `Shell/`)
currently using the [limine](https://limine-bootloader.org/) bootloader
