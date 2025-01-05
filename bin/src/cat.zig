const libc = @import("libc");
const printf = libc.stdio.zprintf;
const File = libc.stdio.File;
pub const panic = libc.panic;

pub fn main() !void {
    const args = libc.sys.args();
    if (args.count() < 2) {
        try printf("expected filename to cat\n", .{});
        return error.NotEnoughArguments;
    }

    const filename = args.nth(1).?;

    const file = try File.open(filename, .{ .read = true });
    defer file.close();

    const data = try file.reader().readUntilEOF();
    try printf("%.*s\n", .{ data.len, data.ptr });
}

comptime {
    _ = libc;
}
