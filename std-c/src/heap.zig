const std = @import("std");
const libc = @import("libc");
const builtin = @import("builtin");

pub const c_allocator = std.mem.Allocator{
    .ptr = undefined,
    .vtable = &c_allocator_vtable,
};

const c_allocator_vtable = std.mem.Allocator.VTable{
    .alloc = c_alloc,
    .resize = c_realloc,
    .free = c_free,
};

fn c_alloc(
    _: *anyopaque,
    len: usize,
    _: u8,
    _: usize,
) ?[*]u8 {
    const results = libc.stdlib.zalloc(u8, len) catch return null;
    return results.ptr;
}

fn c_free(
    _: *anyopaque,
    ptr: []u8,
    _: u8,
    _: usize,
) void {
    libc.stdlib.free(@ptrCast(ptr.ptr));
}

fn c_realloc(
    _: *anyopaque,
    ptr: []u8,
    _: u8,
    new_len: usize,
    _: usize,
) bool {
    const new_ptr = libc.stdlib.zrealloc(u8, ptr, new_len) orelse return false;

    if (new_ptr.ptr != ptr.ptr) {
        libc.stdlib.free(ptr.ptr);
        return false;
    }

    return true;
}
