# SafaOS Features
This is an incomplete attempt to make a roadmap and a list of current features.

If you want a more detailed overview of the current api features checkout the [safa-api docs](https://docs.rs/safa-api/latest/safa_api/).

## Program Ports
Some cool and useful programs that were ported to `SafaOS`:
- [X] doomgeneric
- [X] quake2generic
- [X] ccleste
- [X] [SDL2](https://github.com/SafaOS/recipes/tree/main/ports/SDL2)
- [X] [SDL2_mixer](https://github.com/SafaOS/recipes/tree/main/ports/SDL2_mixer)
- more can be found in [recipes](https://github.com/SafaOS/recipes/)

## Library Ports
Some useful libraries that were ported to `SafaOS`:
- [X] Rust's [libstd](https://github.com/SafaOS/safa-rust)
- [X] [libc](https://github.com/SafaOS/libc)

# Architectures
- [X] x86_64
- [X] AArch64 (incomplete only qemu virt #24)

## Userspace & processes
Overview of what processes can currently do:
- [X] Userspace
- [X] ELF-loader
- [X] Environment variables
- [X] Arguments
- [X] IPC
  - [X] Unix Domain Sockets
  - [X] Shared memory
  - [X] VTTYs (similar to PTYs but worse and the design is unfinished...)
- [ ] Signals (will be simulated)
- [X] MLFQ Scheduler with SMP
- [X] ELF Thread Local Storage
- [X] Futexes
- [X] UDP Unix Sockets
- [ ] TCP Unix Sockets

# VFS
Overview of what the VFS can currently do & ported file systems:
- [X] Creating & Opening & Deleting files
- [X] Operations: reading, writing, truncating, ioctl, buffering (sync), iterating directories
- [X] TmpFS
- [X] `rod:/` fs, similar to procfs but isn't process specific
- [X] unix-like devices FS `dev:/` (TmpFS under the hood)

# Devices & Drivers
- [X] XHCI Driver
- [X] PS2 Keyboard Driver (x86_64 only)
- [X] USB Keyboard Driver
- [X] Serial Device: `dev:/ss`
- [X] TTY Emulator: `dev:/tty` (to be removed)
- [X] Memory mappable Framebuffer Device: `dev:/fb`
- [X] AC97 Audio driver

# Bootloaders
- [X] UEFI Limine
- [X] BIOS Limine
- [ ] Generic abstraction interface over bootloaders
