const std_c = @import("std-c");
const std = @import("std");

const panic = std_c.panic;
const print = std_c.print;

const Error = std_c.Error;

pub fn main() Error!void {
    var args = std_c.sys.args();

    if (args.count() < 2) return error.NotEnoughArguments;

    const errstr = args.nth(1).?;

    const errnum = std.fmt.parseInt(u16, errstr, 10) catch return error.ArgumentOutOfDomain;
    const err: Error = @errorCast(@errorFromInt(errnum));

    const name = @errorName(err);

    print("{s}\n", .{name});
}

comptime {
    _ = std_c;
}
