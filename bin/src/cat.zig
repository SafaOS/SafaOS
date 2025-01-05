const libc = @import("libc");
const printf = libc.stdio.zprintf;
const File = libc.stdio.File;
pub const panic = libc.panic;

pub fn main() !void {
    const args = libc.sys.args();
    var data: []u8 = undefined;

    if (args.count() > 1) {
        const filename = args.nth(1).?;

        const file = try File.open(filename, .{ .read = true });
        defer file.close();

        data = try file.reader().readUntilEOF();
    } else {
        data = try libc.stdio.zgetline();
    }

    try printf("%.*s\n", .{ data.len, data.ptr });
}

comptime {
    _ = libc;
}
