const std = @import("std");
const std_c = @import("std-c");

const print = std_c.print;
const Errno = std_c.Error;
const Dir = std_c.dirent.DIR;

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

    while (cwd.next()) |ent| {
        if (!raw_output) {
            if (ent.kind == 1)
                print("\x1B[38;2;0;100;255m{s}\n\x1B[0m", .{ent.name[0..ent.name_length]})
            else if (ent.kind == 2)
                print("\x1B[38;2;255;0;0m{s}\n\x1B[0m", .{ent.name[0..ent.name_length]})
            else
                print("{s}\n", .{ent.name[0..ent.name_length]});
        } else print("{s}\n", .{ent.name[0..ent.name_length]});
    }
}

comptime {
    _ = std_c;
}
