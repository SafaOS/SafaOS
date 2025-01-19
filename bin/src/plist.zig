const std_c = @import("std-c");
const std = @import("std");
const sysinfo = std_c.sys.utils.zsysinfo;
const allocator = std_c.heap.c_allocator;
const pcollect = std_c.sys.utils.zpcollect;
const print = std_c.print;
const Dir = std_c.dirent.DIR;
const File = std_c.stdio.File;

const ProccessInfo = struct {
    name: []const u8,
    status: []const u8,
    pid: u32,
    ppid: u32,
};

pub fn main() !void {
    const proc_dir = try Dir.open("proc:/");
    defer proc_dir.close();

    print("name:  pid  ppid  status\n", .{});
    while (proc_dir.next()) |entry| {
        if (std.fmt.parseInt(u32, entry.name[0..entry.name_length], 10) catch null) |pid| {
            const path = try std.fmt.allocPrint(allocator, "proc:/{d}/info", .{pid});
            defer allocator.free(path);

            const file = try File.open(path, .{ .read = true });
            defer file.close();

            const info = try file.reader().readUntilEOF();

            const process_info = try std.json.parseFromSlice(ProccessInfo, allocator, info, .{ .ignore_unknown_fields = true });
            defer process_info.deinit();

            const process = process_info.value;

            print("\x1B[38;2;0;255;0m{s}\x1B[0m:  {}  {}  {s}\n", .{ process.name, process.pid, process.ppid, process.status });
        }
    }
}

comptime {
    _ = std_c;
}
