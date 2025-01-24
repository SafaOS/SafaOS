const std_c = @import("std-c");
const std = @import("std");
const sys = std_c.sys;

const print = std_c.print;
const StdinReader = std_c.StdinReader;
pub const allocator = std_c.heap.c_allocator;

const Lexer = @import("Lexer.zig");
const repl = @import("repl.zig");

const Error = std_c.Error;
pub const panic = std_c.panic;

const environment = @import("environment.zig");
var ret: u64 = 0;

pub fn prompt() Error!void {
    const cwd_buffer = try allocator.alloc(u8, 1024);
    defer allocator.free(cwd_buffer);

    const cwd_len = try sys.io.zgetcwd(cwd_buffer);

    print("\x1B[38;2;255;0;193m{s}\x1B[0m ", .{cwd_buffer[0..cwd_len]});

    if (ret != 0) {
        print("\x1B[38;2;255;0;0m[{}]\x1B[0m ", .{ret});
    }

    print("# ", .{});
}

pub fn run(line: []const u8) Error!void {
    var tokens = std.ArrayList(Lexer.Token).init(allocator);
    defer tokens.deinit();

    var lexer = Lexer.init(line);
    while (lexer.next()) |token| {
        try tokens.append(token);
    }
    if (tokens.items.len < 1) return;

    const name = tokens.items[0].asString();
    ret = repl.repl(tokens.items) catch |err| blk: {
        const err_name = @errorName(err);
        print("failed to execute {s}, error: {s}\n", .{ name, err_name });
        break :blk 0;
    };
}

pub fn main() Error!void {
    print("\x1B[38;2;255;192;203m", .{});
    print(
        \\  ,---.             ,---.           ,-----.   ,---.   
        \\ '   .-'   ,--,--. /  .-'  ,--,--. '  .-.  ' '   .-'  
        \\ `.  `-.  ' ,-.  | |  `-, ' ,-.  | |  | |  | `.  `-.  
        \\ .-'    | \ '-'  | |  .-' \ '-'  | '  '-'  ' .-'    | 
        \\ `-----'   `--`--' `--'    `--`--'  `-----'  `-----'  
    , .{});

    print("\n\x1B[38;2;200;200;200m", .{});
    print(
        \\| Welcome to SafaOS!
        \\| you are currently in ram:/, a playground
        \\| init ramdisk has been mounted at sys:/
        \\| sys:/bin is avalible in your PATH check it out for some binaries
        \\| the command `help` will provide a list of builtin commands and some terminal usage guide
    , .{});

    print("\x1B[0m\n", .{});

    try environment.init();

    while (true) {
        try prompt();
        const line = try StdinReader.readUntilDelimiterAlloc(
            allocator,
            '\n',
            std.math.maxInt(usize),
        );
        defer allocator.free(line);

        try run(line);
    }
}

comptime {
    _ = std_c;
}
