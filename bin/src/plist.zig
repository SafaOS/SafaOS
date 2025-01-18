const std_c = @import("std-c");

const sysinfo = std_c.sys.utils.zsysinfo;
const allocator = std_c.heap.c_allocator;
const pcollect = std_c.sys.utils.zpcollect;
const print = std_c.print;

pub fn main() !void {
    const info = try sysinfo();
    const processes = try allocator.alloc(std_c.sys.raw.ProcessInfo, info.processes_count);
    defer allocator.free(processes);

    _ = try pcollect(processes);

    print("name:  pid  ppid\n", .{});
    for (processes) |process| {
        print("\x1B[38;2;0;255;0m{s}\x1B[0m:  {}  {}\n", .{ process.name, process.pid, process.ppid });
    }
}

comptime {
    _ = std_c;
}
