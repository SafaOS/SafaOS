const std_c = @import("std-c");
const print = std_c.print;
const Error = std_c.Error;

pub fn main() Error!void {
    var args = std_c.sys.args();

    _ = args.next();

    if (args.next()) |arg| {
        print("{s}", .{arg});
    }

    while (args.next()) |arg| {
        print(" {s}", .{arg});
    }

    print("\n", .{});
}

comptime {
    _ = std_c;
}
