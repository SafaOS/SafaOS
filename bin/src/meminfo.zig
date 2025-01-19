const std_c = @import("std-c");
const std = @import("std");

const sysinfo = std_c.sys.utils.zsysinfo;
const print = std_c.print;
const File = std_c.stdio.File;
const allocator = std_c.heap.c_allocator;

const Mode = enum {
    Bytes,
    KiB,
    MiB,
    Verbose,
};

const MemInfo = struct {
    total: usize,
    free: usize,
    used: usize,
};

pub fn main() !void {
    const info_file = try File.open("proc:/meminfo", .{ .read = true });
    defer info_file.close();

    const info_str = try info_file.reader().readUntilEOF();

    const parsed_info = try std.json.parseFromSlice(MemInfo, allocator, info_str, .{ .allocate = .alloc_if_needed, .ignore_unknown_fields = true });
    defer parsed_info.deinit();

    const info = parsed_info.value;

    var mode: Mode = .Verbose;
    var args = std_c.sys.args();

    while (args.next()) |arg| {
        if (std.mem.eql(u8, arg, "-b")) {
            mode = .Bytes;
        } else if (std.mem.eql(u8, arg, "-k")) {
            mode = .KiB;
        } else if (std.mem.eql(u8, arg, "-m")) {
            mode = .MiB;
        }
    }

    switch (mode) {
        .Bytes => {
            print("{}B/{}B\n", .{ info.used, info.total });
        },
        .KiB => {
            print("{}KiB/{}KiB\n", .{ info.used / 1024, info.total / 1024 });
        },
        .MiB => {
            print("{}MiB/{}MiB\n", .{ info.used / 1024 / 1024, info.total / 1024 / 1024 });
        },

        .Verbose => {
            print("memory info:\n", .{});
            print("{}B used of {}B, {}B usable\n", .{ info.used, info.total, info.free });

            print("{}KiBs used of {}KiBs, {}KiBs usable\n", .{ info.used / 1024, info.total / 1024, info.free / 1024 });

            print("{}MiBs used of {}MiBs, {}MiBs usable\n", .{ info.used / 1024 / 1024, info.total / 1024 / 1024, info.free / 1024 / 1024 });
        },
    }
}

comptime {
    _ = std_c;
}
