const std_c = @import("std-c");

const File = std_c.stdio.File;
const print = std_c.print;
const panic = std_c.panic;

pub fn main() !void {
    const args = std_c.sys.args();
    if (args.count() < 3) {
        print("expected filename to write to, and data to write\n", .{});
        return error.NotEnoughArguments;
    }
    const filename = args.nth(1).?;
    const data = args.nth(2).?;

    const file = try File.open(filename, .{ .write = true });
    defer file.close();

    const writer = file.writer();
    try writer.write(data);
}
comptime {
    _ = std_c;
}
