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
    is_alive: bool,
    pid: u32,
    ppid: u32,
};

pub fn main() !void {
    const proc_dir = try Dir.open("proc:/");
    defer proc_dir.close();

    var longest_name: usize = 0;
    const longest_status: usize = 5;
    var longest_pid: usize = 4;

    var processes = std.ArrayList(std.json.Parsed(ProccessInfo)).init(allocator);
    defer processes.deinit();

    while (proc_dir.next()) |entry| {
        if (std.fmt.parseInt(u32, entry.name[0..entry.name_length], 10) catch null) |pid| {
            const longest_pid_val = std.math.pow(usize, 10, longest_pid);

            if (pid > longest_pid_val * 10)
                longest_pid = std.math.log10(longest_pid_val * 10);

            const path = try std.fmt.allocPrint(allocator, "proc:/{d}/info", .{pid});
            defer allocator.free(path);

            const file = try File.open(path, .{ .read = true });
            defer file.close();

            const info = try file.reader().readAllAlloc(allocator, std.math.maxInt(usize));

            const process_info = try std.json.parseFromSlice(ProccessInfo, allocator, info, .{ .ignore_unknown_fields = true });
            try processes.append(process_info);
            const process = process_info.value;

            if (process.name.len > longest_name)
                longest_name = process.name.len;
        }
    }

    print("\x1B[32m{[name]s:<[longest_name]}\x1B[0m:  \x1B[31m{[pid]s:<[pid_align]}  {[ppid]s:<[pid_align]}  \x1B[33m{[status]s:<5}\x1b[0m\n", .{
        .longest_name = longest_name,
        .name = "name",
        .pid = "pid",
        .ppid = "ppid",
        .status = "alive",
        .pid_align = longest_pid,
    });

    print("{s:-<[width]}\n", .{ .nothing = "", .width = longest_name + longest_pid + longest_status + 3 + 4 + 6 });

    for (processes.items) |process_info| {
        const process = process_info.value;
        print("\x1B[32m{[name]s:<[longest_name]}\x1B[0m:  \x1B[31m{[pid]d:<[pid_align]}  {[ppid]d:<[pid_align]}  \x1B[33m{[is_alive]:<5}\x1b[0m\n", .{
            .longest_name = longest_name,
            .name = process.name,
            .pid = process.pid,
            .ppid = process.ppid,
            .is_alive = process.is_alive,
            .pid_align = longest_pid,
        });
    }
}

comptime {
    _ = std_c;
}
