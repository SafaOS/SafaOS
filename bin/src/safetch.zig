const libc = @import("libc");
const print = libc.stdio.zprintf;

pub fn main() !void {
    // fetching info
    const sysinfo = libc.sys.utils.sysinfo().?;
    const total_memory = sysinfo.total_mem / 1024 / 1024;
    const used_memory = sysinfo.used_mem / 1024 / 1024;

    // draw the logo
    const logo_file = try libc.stdio.FILE.open("sys:/logo.txt", .{ .read = true });
    defer logo_file.close();

    const logo = try logo_file.reader().readUntilEOF();
    try print("%.*s\n", .{ logo.len, logo.ptr });

    // for now we don't really have a way to easily figure out the logo's width + height and the terminal's width + height so we just hardcode it
    // start drawing from the end of the start of the logo
    try print("\x1b[11A", .{});

    try print("\x1b[31C\x1b[31mroot\x1b[0m@\x1b[31mlocalhost\x1b[0m\n\n", .{});
    try print("\x1b[31C\x1b[31mOS:\x1b[0m SafaOS (UNKNOWN)\n", .{});
    try print("\x1b[31C\x1b[31mKernel:\x1b[0m Snowball (UNKNOWN)\n", .{});
    try print("\x1b[31C\x1b[31mTerminal:\x1b[0m dev:/tty\n", .{});
    try print("\x1b[31C\x1b[31mMemory:\x1b[0m %luMiB / %luMiB\n\n", .{ used_memory, total_memory });

    try print("\x1b[31C\x1b[30m\x1b[40m   \x1b[31m\x1b[41m   \x1b[32m\x1b[42m   \x1b[33m\x1b[43m   \x1b[34m\x1b[44m   \x1b[35m\x1b[45m   \x1b[36m\x1b[46m   \x1b[37m\x1b[47m   \x1b[m\n", .{});
    try print("\x1b[31C\x1b[90m\x1b[100m   \x1b[91m\x1b[101m   \x1b[92m\x1b[102m   \x1b[93m\x1b[103m   \x1b[94m\x1b[104m   \x1b[95m\x1b[105m   \x1b[96m\x1b[106m   \x1b[97m\x1b[107m   \x1b[m\n", .{});

    try print("\x1b[2B", .{});
}

comptime {
    _ = libc;
}
