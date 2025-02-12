//! this is a wrapper around the libc module which provides a few functions and types as an alternative to `LinkLibC`
//! for example the `heap` module provides a `c_allocator` which is a `std.mem.Allocator` that uses the libc allocator
//! to allocate memory as an alternative to [`std.heap.c_allocator`] ([`heap.c_allocator`])
//! please do this before using any of the functions in this module in the `main.zig` file if any (to make sure that main is the entry point)
//! ```zig
//! // note that this is not necessary if you are never going to panic
//! const panic = std_c.panic;
//! comptime {
//!    _ = @import("std-c");
//! }
//! ```
const libc = @import("libc");
const std = @import("std");

pub const panic = libc.panic;
pub const print = libc.stdio.zprintf;

pub extern var stdin: libc.stdio.File;
pub extern var stdout: libc.stdio.File;

comptime {
    _ = libc;
}

pub const heap = @import("heap.zig");
pub const sys = libc.sys;
pub const syscalls = libc.syscalls;
pub const Error = sys.errno.Error;

pub const stdio = libc.stdio;
pub const dirent = libc.dirent;
