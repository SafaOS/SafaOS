//! this file should contain the raw syscall functions
const raw = @import("raw.zig");

inline fn syscall0(number: usize) usize {
    return asm volatile ("int $0x80"
        : [ret] "={rax}" (-> usize),
        : [number] "{rax}" (number),
        : "rcx", "r11"
    );
}

inline fn syscall1(number: usize, arg1: usize) usize {
    return asm volatile ("int $0x80"
        : [ret] "={rax}" (-> usize),
        : [number] "{rax}" (number),
          [arg1] "{rdi}" (arg1),
        : "rcx", "r11"
    );
}

inline fn syscall3(number: usize, arg1: usize, arg2: usize, arg3: usize) usize {
    return asm volatile ("int $0x80"
        : [ret] "={rax}" (-> usize),
        : [number] "{rax}" (number),
          [arg1] "{rdi}" (arg1),
          [arg2] "{rsi}" (arg2),
          [arg3] "rdx" (arg3),
        : "rcx", "r11"
    );
}

inline fn syscall4(number: usize, arg1: usize, arg2: usize, arg3: usize, arg4: usize) usize {
    return asm volatile ("int $0x80"
        : [ret] "={rax}" (-> usize),
        : [number] "{rax}" (number),
          [arg1] "{rdi}" (arg1),
          [arg2] "{rsi}" (arg2),
          [arg3] "{rdx}" (arg3),
          [arg4] "{rcx}" (arg4),
        : "r8", "r11"
    );
}
inline fn syscall6(number: usize, arg1: usize, arg2: usize, arg3: usize, arg4: usize, arg5: usize, arg6: usize) usize {
    return asm volatile ("int $0x80"
        : [ret] "={rax}" (-> usize),
        : [number] "{rax}" (number),
          [arg1] "{rdi}" (arg1),
          [arg2] "{rsi}" (arg2),
          [arg3] "{rdx}" (arg3),
          [arg4] "{rcx}" (arg4),
          [arg5] "{r8}" (arg5),
          [arg6] "{r9}" (arg6),
        : "r10", "r11"
    );
}
pub inline fn exit(code: usize) void {
    _ = syscall1(0, code);
}

pub inline fn yield() void {
    _ = syscall1(1);
}

pub inline fn open(path: *const u8, len: usize, fd: *usize) usize {
    return syscall3(2, @intFromPtr(path), len, @intFromPtr(fd));
}

pub inline fn write(fd: usize, ptr: *const u8, len: usize) usize {
    return syscall4(3, fd, @intFromPtr(ptr), len, 0);
}

pub inline fn read(fd: usize, ptr: *u8, len: usize, num_read: *usize) usize {
    return syscall4(4, fd, @intFromPtr(ptr), len, @intFromPtr(num_read));
}

pub inline fn close(fd: usize) usize {
    return syscall1(5, fd);
}

pub inline fn create(path_ptr: *const u8, path_len: usize) usize {
    return syscall3(6, @intFromPtr(path_ptr), path_len, 0);
}

pub inline fn createdir(path_ptr: *const u8, path_len: usize) usize {
    return syscall3(7, @intFromPtr(path_ptr), path_len, 0);
}

pub inline fn diriter_open(dir_ri: usize, dest_diriter: *usize) usize {
    return syscall3(8, dir_ri, @intFromPtr(dest_diriter), 0);
}

pub inline fn diriter_close(diriter: usize) usize {
    return syscall1(9, diriter);
}

pub inline fn diriter_next(diriter: usize, direntry: *raw.DirEntry) usize {
    return syscall3(10, diriter, @intFromPtr(direntry), 0);
}

pub inline fn wait(pid: usize) usize {
    return syscall1(11, pid);
}

pub inline fn fstat(ri: usize, direntry: *raw.DirEntry) usize {
    return syscall3(12, ri, @intFromPtr(direntry), 0);
}

pub inline fn spawn(elf_ptr: *const u8, elf_len: usize, config: *const raw.SpawnConfig, dest_pid: *u64) usize {
    return syscall4(13, @intFromPtr(elf_ptr), elf_len, @intFromPtr(config), @intFromPtr(dest_pid));
}

pub inline fn chdir(path_ptr: [*]const u8, path_len: usize) usize {
    return syscall3(14, @intFromPtr(path_ptr), path_len, 0);
}

pub inline fn getcwd(ptr: [*]const u8, len: usize, dest_len_got: *usize) usize {
    return syscall3(15, @intFromPtr(ptr), len, @intFromPtr(dest_len_got));
}

pub inline fn info(ptr: *raw.SysInfo) usize {
    return syscall1(16, @intFromPtr(ptr));
}

pub inline fn pcollect(ptr: *raw.ProcessInfo, len: usize) usize {
    return syscall3(17, @intFromPtr(ptr), len, 0);
}

pub inline fn sbrk(amount: isize) ?*u8 {
    return @ptrFromInt(syscall1(18, @bitCast(amount)));
}

pub inline fn pspawn(path_ptr: *const u8, path_len: usize, config: *const raw.SpawnConfig, dest_pid: *u64) usize {
    return syscall4(19, @intFromPtr(path_ptr), path_len, @intFromPtr(config), @intFromPtr(dest_pid));
}

pub inline fn shutdown() noreturn {
    _ = syscall0(20);
    unreachable;
}

pub inline fn reboot() noreturn {
    _ = syscall0(21);
    unreachable;
}
