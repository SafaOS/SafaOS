const std = @import("std");
const std_c = @import("std-c");

const print = std_c.print;
const Errno = std_c.Error;
const Dir = std_c.dirent.DIR;
const allocator = std_c.heap.c_allocator;

const Name = struct {
    raw: [128]u8,
    len: usize,

    fn init(raw: [128]u8, len: usize) Name {
        return Name{
            .raw = raw,
            .len = len,
        };
    }

    fn toSlice(self: *const Name) []const u8 {
        return self.raw[0..self.len];
    }
};

const dirColor = "\x1B[34m";
const fileColor = "";
const otherColor = "\x1B[31m";

pub fn main() !void {
    var args = std_c.sys.args();
    const cwd = try Dir.open(".");
    defer cwd.close();

    var raw_output = false;
    while (args.next()) |arg| {
        if (std.mem.eql(u8, arg, "--raw")) {
            raw_output = true;
        }
    }

    var dirs = std.ArrayList(Name).init(allocator);
    var files = std.ArrayList(Name).init(allocator);
    var others = std.ArrayList(Name).init(allocator);
    defer {
        dirs.deinit();
        files.deinit();
        others.deinit();
    }

    while (cwd.next()) |ent| {
        if (ent.kind == 0)
            try files.append(Name.init(ent.name, ent.name_length))
        else if (ent.kind == 1)
            try dirs.append(Name.init(ent.name, ent.name_length))
        else
            try others.append(Name.init(ent.name, ent.name_length));
    }

    for (dirs.items) |dir| {
        if (raw_output) {
            print("{s}\n", .{dir.toSlice()});
        } else print("{s}{s}\x1B[0m\n", .{ dirColor, dir.toSlice() });
    }

    for (files.items) |file| {
        if (raw_output) {
            print("{s}\n", .{file.toSlice()});
        } else print("{s}{s}\x1B[0m\n", .{ fileColor, file.toSlice() });
    }

    for (others.items) |other| {
        if (raw_output) {
            print("{s}\n", .{other.toSlice()});
        } else print("{s}{s}\x1B[0m\n", .{ otherColor, other.toSlice() });
    }
}

comptime {
    _ = std_c;
}
