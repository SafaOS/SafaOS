const std_c = @import("std-c");
const std = @import("std");
const sys = std_c.sys;

const allocator = @import("main.zig").allocator;

const Slice = sys.raw.Slice;
const Error = std_c.Error;

const Token = @import("Lexer.zig").Token;
const Dir = std_c.dirent.DIR;

const zpspawn = sys.utils.zpspwan;
const wait = sys.utils.wait;
const environment = @import("environment.zig");

const ExecuteBuiltin = @import("builtin.zig").executeBuiltin;

fn spawn(name: []const u8, argv: []const Slice(u8)) Error!u64 {
    var path_var = try environment.get_path();
    defer path_var.deinit();

    for (path_var.items) |path| {
        var it = try Dir.open(path);
        defer it.close();

        while (it.next()) |entry| {
            const entry_name = entry.name[0..entry.name_length];

            if (std.mem.eql(u8, entry_name, name)) {
                var full_path = std.ArrayList(u8).init(allocator);
                defer full_path.deinit();

                try full_path.appendSlice(path);
                try full_path.appendSlice("/");
                try full_path.appendSlice(entry_name);

                const pid = zpspawn(full_path.items, argv, name);
                return pid;
            }
        }
    }

    return error.NoSuchAFileOrDirectory;
}

pub fn repl(tokens: []const Token) Error!usize {
    if (tokens.len == 0) return 0;

    const argv = try allocator.alloc(Slice(u8), tokens.len);
    defer allocator.free(argv);

    for (tokens, 0..) |token, i| {
        const string = token.asString();

        argv[i] = .{ .ptr = string.ptr, .len = string.len };
    }

    const name = argv[0];
    const results = ExecuteBuiltin(name, argv) orelse {
        const pid = try spawn(name.ptr[0..name.len], argv);
        return wait(pid);
    };
    return results;
}
