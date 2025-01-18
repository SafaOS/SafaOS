const std_c = @import("std-c");
const std = @import("std");

const print = std_c.print;
const File = std_c.stdio.File;

pub const panic = std_c.panic;

pub fn main() !void {
    const args = std_c.sys.args();
    var data: []u8 = undefined;

    if (args.count() > 1) {
        const filename = args.nth(1).?;

        const file = try File.open(filename, .{ .read = true });
        defer file.close();

        data = try file.reader().readUntilEOF();
    } else {
        const StdinReader = std_c.StdinReader;

        const stdin_data = try StdinReader.readUntilDelimiterOrEofAlloc(std_c.heap.c_allocator, '\n', std.math.maxInt(usize));
        data = stdin_data.?;
    }

    print("{s}\n", .{data});
}

comptime {
    _ = std_c;
}
