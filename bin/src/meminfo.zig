const std_c = @import("std-c");
const std = @import("std");

const sysinfo = std_c.sys.utils.zsysinfo;
const print = std_c.print;

const Mode = enum {
    Bytes,
    KiB,
    MiB,
    Verbose,
};
pub fn main() !void {
    const info = try sysinfo();
    const mem_ava: usize = info.total_mem - info.used_mem;

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
            print("{}B/{}B\n", .{ info.used_mem, info.total_mem });
        },
        .KiB => {
            print("{}KiB/{}KiB\n", .{ info.used_mem / 1024, info.total_mem / 1024 });
        },
        .MiB => {
            print("{}MiB/{}MiB\n", .{ info.used_mem / 1024 / 1024, info.total_mem / 1024 / 1024 });
        },

        .Verbose => {
            print("memory info:\n", .{});
            print("{}B used of {}B, {}B usable\n", .{ info.used_mem, info.total_mem, mem_ava });

            print("{}KiBs used of {}KiBs, {}KiBs usable\n", .{ info.used_mem / 1024, info.total_mem / 1024, mem_ava / 1024 });

            print("{}MiBs used of {}MiBs, {}MiBs usable\n", .{ info.used_mem / 1024 / 1024, info.total_mem / 1024 / 1024, mem_ava / 1024 / 1024 });
        },
    }
}

comptime {
    _ = std_c;
}
