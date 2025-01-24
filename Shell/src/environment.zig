const std = @import("std");
const allocator = @import("main.zig").allocator;

pub const EnvironmentVariable = struct {
    name: []const u8,
    value: []const u8,
};

var environment: std.ArrayList(EnvironmentVariable) = undefined;

pub fn init() !void {
    environment = std.ArrayList(EnvironmentVariable).init(allocator);
    try add_environment_variable("PATH", "sys:/bin");
}

pub fn add_environment_variable(name: []const u8, value: []const u8) !void {
    try environment.append(.{ .name = name, .value = value });
}

pub fn get_environment_variable(name: []const u8) ?[]const u8 {
    for (environment.items) |env| {
        if (std.mem.eql(u8, env.name, name)) {
            return env.value;
        }
    }
    return null;
}

pub fn get_path() !std.ArrayList([]const u8) {
    var path = std.ArrayList([]const u8).init(allocator);
    // adding current dir
    try path.append(".");

    const path_env = get_environment_variable("PATH") orelse return path;

    var current_start: usize = 0;
    for (path_env, 0..) |path_part, i| {
        if (path_part == ';') {
            try path.append(path_env[current_start..i]);
            current_start = i + 1;
        }

        if (i == path_env.len - 1) {
            try path.append(path_env[current_start..]);
        }
    }

    return path;
}
