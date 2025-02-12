const std_c = @import("std-c");
const std = @import("std");
const print = std_c.print;

const File = std_c.stdio.File;
const allocator = std_c.heap.c_allocator;

const MemInfo = struct {
    total: usize,
    free: usize,
    used: usize,
};

const CpuInfo = struct {
    vendor_id: []const u8,
    model: []const u8,
};

const KernelInfo = struct {
    name: []const u8,
    version: []const u8,
    compile_time: []const u8,
    compile_date: []const u8,
    uptime: u64,
};

fn get_parsed(comptime T: type, comptime path: []const u8) !std.json.Parsed(T) {
    const file = try File.open(path, .{ .read = true });
    defer file.close();

    const str = try file.reader().readAllAlloc(allocator, std.math.maxInt(usize));
    defer allocator.free(str);

    const parsed = try std.json.parseFromSlice(T, allocator, str, .{ .ignore_unknown_fields = true, .allocate = .alloc_always });

    return parsed;
}

/// Prints a field with a value
/// `fmt` formats the field value
/// `field` is the field name
/// `value` is the field value
fn print_field(comptime field: []const u8, comptime fmt: []const u8, value: anytype) void {
    print("\x1b[31C\x1b[31m" ++ field ++ "\x1b[0m " ++ fmt ++ "\n", value);
}

fn get_uptime(uptime_in_seconds: u64, uptime_buffer: []u8) []const u8 {
    if (uptime_in_seconds < 60)
        return std.fmt.bufPrint(uptime_buffer, "{d}s", .{uptime_in_seconds}) catch unreachable
    else if (uptime_in_seconds < 60 * 60) {
        const minutes_uptime = uptime_in_seconds / 60;
        const seconds_uptime = uptime_in_seconds % 60;
        return std.fmt.bufPrint(uptime_buffer, "{d}m{d}s", .{ minutes_uptime, seconds_uptime }) catch unreachable;
    } else {
        const hours_uptime = uptime_in_seconds / 60 / 60;
        const minutes_uptime = uptime_in_seconds / 60 % 60;
        return std.fmt.bufPrint(uptime_buffer, "{d}h{d}m", .{ hours_uptime, minutes_uptime }) catch unreachable;
    }
}

pub fn main() !void {
    const parsed_meminfo = try get_parsed(MemInfo, "proc:/meminfo");
    defer parsed_meminfo.deinit();
    const meminfo = parsed_meminfo.value;

    const parsed_cpuinfo = try get_parsed(CpuInfo, "proc:/cpuinfo");
    defer parsed_cpuinfo.deinit();

    const parsed_kernelinfo = try get_parsed(KernelInfo, "proc:/kernelinfo");
    defer parsed_kernelinfo.deinit();

    const kernelinfo = parsed_kernelinfo.value;
    const cpuinfo = parsed_cpuinfo.value;

    // fetching info
    const total_memory = meminfo.total / 1024 / 1024;
    const used_memory = meminfo.used / 1024 / 1024;

    const uptime_in_seconds = kernelinfo.uptime / 1000;

    var uptime_buffer: [64]u8 = undefined;
    const uptime = get_uptime(uptime_in_seconds, &uptime_buffer);

    // draw the logo
    const logo_file = try File.open("sys:/logo.txt", .{ .read = true });
    defer logo_file.close();

    const logo = try logo_file.reader().readAllAlloc(allocator, std.math.maxInt(usize));
    defer allocator.free(logo);

    print("{s}\n", .{logo});

    // for now we don't really have a way to easily figure out the logo's width + height and the terminal's width + height so we just hardcode it
    // start drawing from the end of the start of the logo
    print("\x1b[11A", .{});

    print_field("root@localhost", "", .{});
    print_field("OS:", "SafaOS", .{});
    print_field("Kernel:", "{s} (v{s} built on {s})", .{ kernelinfo.name, kernelinfo.version, kernelinfo.compile_date });
    print_field("Uptime:", "{s}", .{uptime});
    print_field("Terminal:", "dev:/tty", .{});
    print_field("CPU:", "{s}", .{cpuinfo.model});
    print_field("Memory:", "{}MiB / {}MiB\n", .{ used_memory, total_memory });

    print("\x1b[31C\x1b[30m\x1b[40m   \x1b[31m\x1b[41m   \x1b[32m\x1b[42m   \x1b[33m\x1b[43m   \x1b[34m\x1b[44m   \x1b[35m\x1b[45m   \x1b[36m\x1b[46m   \x1b[37m\x1b[47m   \x1b[m\n", .{});
    print("\x1b[31C\x1b[90m\x1b[100m   \x1b[91m\x1b[101m   \x1b[92m\x1b[102m   \x1b[93m\x1b[103m   \x1b[94m\x1b[104m   \x1b[95m\x1b[105m   \x1b[96m\x1b[106m   \x1b[97m\x1b[107m   \x1b[m", .{});

    print("\x1b[2B\n", .{});
}

comptime {
    _ = std_c;
}
