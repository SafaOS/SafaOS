const sbrk = @import("sys/mem.zig").zsbrk;
const memcpy = @import("string.zig").memcpy;

const INIT_SIZE = 4096;
const MALLOC_SIZE_ALIGN = 16;
const Chunk = extern struct {
    size: usize,
    free: bool,
    data_off: [7]u8,
    pub fn data(self: *@This()) [*]u8 {
        const ptr: [*]u8 = @ptrCast(&self.data_off[self.data_off.len - 1]);
        return ptr + 1;
    }
};

pub export var head: ?*Chunk = null;

fn brk() *anyopaque {
    return sbrk(0) catch unreachable;
}

fn align_up(value: usize, alignment: usize) usize {
    return (value + (alignment - 1)) & ~(alignment - 1);
}

/// increases heap size and adds a free Chunk with size `size` at the end
fn add_free(size: usize) !*Chunk {
    const ptr: *Chunk = @ptrCast(@alignCast(brk()));
    _ = try sbrk(@intCast(size + @sizeOf(Chunk)));

    ptr.size = size;
    ptr.free = true;
    return ptr;
}

pub export fn __malloc__init__() void {
    head = add_free(INIT_SIZE) catch null;
}

/// finds a free chunk starting from `head`
fn find_free(size: usize) ?*Chunk {
    var current = head orelse return null;
    const end = brk();

    while (@intFromPtr(current) < @intFromPtr(end)) {
        if (current.size >= size and current.free)
            return current;
        current = @ptrFromInt(@intFromPtr(current) + @sizeOf(Chunk) + current.size);
    }

    return null;
}

pub export fn malloc(size: usize) ?*anyopaque {
    const asize = if (size != 0) align_up(size, MALLOC_SIZE_ALIGN) else MALLOC_SIZE_ALIGN;
    var block = find_free(asize);

    // attempt to increase heap size
    if (block == null)
        block = add_free(asize) catch return null;

    // divide block
    if (block.?.size > asize) {
        // diff is the bigger block
        const diff = block.?.size - asize;
        // diff is able to hold a block of it's own + MALLOC_SIZE_ALIGN
        if (diff >= @sizeOf(Chunk) + MALLOC_SIZE_ALIGN) {
            const new_chunk: *Chunk = @ptrCast(@alignCast(block.?.data() + asize));
            new_chunk.free = true;
            new_chunk.size = diff - @sizeOf(Chunk);

            block.?.size = asize;
        }
    }

    block.?.free = false;
    return @ptrCast(block.?.data());
}

pub fn zmalloc(comptime T: type) ?*T {
    return @ptrCast(@alignCast(malloc(@sizeOf(T))));
}

pub fn zalloc(comptime T: type, n: usize) ![]T {
    const allocated = malloc(n * @sizeOf(T)) orelse return error.OutOfMemory;
    const ptr: [*]T = @ptrCast(@alignCast(allocated));
    return ptr[0..n];
}
/// combines free block starting from head
fn anti_fragmentation() void {
    var current = head orelse return;
    while (true) {
        const next: *Chunk = @ptrFromInt(@intFromPtr(current) + current.size + @sizeOf(Chunk));

        if (@intFromPtr(next) == @intFromPtr(brk()))
            break;

        if (next.free and current.free)
            current.size += next.size + @sizeOf(Chunk)
        else if (!next.free)
            break;
        current = next;
    }
}

pub export fn free(ptr: ?*anyopaque) void {
    if (ptr == null)
        return;

    const chunk: *Chunk = @ptrFromInt(@intFromPtr(ptr.?) - @sizeOf(Chunk));
    chunk.free = true;

    // give the chunk back to the os if it is at the end
    if ((@intFromPtr(chunk) + chunk.size) == @intFromPtr(brk()) and chunk != head) {
        const size: isize = @intCast(chunk.size + @sizeOf(Chunk));
        _ = sbrk(-size) catch unreachable;
        return;
    }

    anti_fragmentation();
}

pub export fn realloc(ptr: *anyopaque, size: usize) ?*anyopaque {
    if (size == 0) {
        free(ptr);
        return null;
    }
    const chunk: *Chunk = @ptrFromInt(@intFromPtr(ptr) - @sizeOf(Chunk));

    if (chunk.size < size) {
        // TODO: improve this so it combines with the next block?
        anti_fragmentation();

        const new = malloc(size);
        _ = memcpy(@ptrCast(new), @ptrCast(ptr), chunk.size);
        free(ptr);

        return new;
    }

    return ptr;
}

pub fn zfree(comptime T: type, buffer: []const T) void {
    free(@ptrCast(@constCast(buffer.ptr)));
}

pub fn zrealloc(comptime T: type, buffer: []T, size: usize) ?[]T {
    const ptr = realloc(@ptrCast(buffer.ptr), size);
    if (ptr == null)
        return null;
    const buf: [*]T = @ptrCast(@alignCast(ptr));
    return buf[0..size];
}
