# SafaOS Features
This is an incomplete attempt to make a roadmap and a list of current features.

If you want a more detailed overview of the current api features checkout the [safa-api docs](https://docs.rs/safa-api/latest/safa_api/).

## Program Ports
Some cool and useful programs that were ported to `SafaOS`:
- [X] [lua](https://github.com/ObserverUnit/SafaOS-lua/tree/v5.4)
- [ ] Doom

## Library Ports
Some useful libraries that were ported to `SafaOS`:
- [X] Rust's [libstd](https://github.com/SafaOS/rust/tree/stable)
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
- [ ] IPC
- [ ] Signals
- [X] Threads with priorities (no load balancing for now)
- [X] ELF Thread Local Storage
- [X] Futexes
- [X] SMP

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
- [ ] Framebuffer Device: `dev:/fb`

# Bootloaders
- [X] UEFI Limine
- [ ] BIOS Limine
- [ ] Generic abstraction interface over bootloaders
