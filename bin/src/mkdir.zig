const std_c = @import("std-c");

const io = std_c.sys.io;
const print = std_c.print;

pub fn main() !void {
    var args = std_c.sys.args();

    if (args.count() < 2) {
        print("expected at least the name of the directory to make\n", .{});
        return error.NotEnoughArguments;
    }

    const path = args.nth(1).?;
    try io.zcreatedir(path);
}

comptime {
    _ = std_c;
}
