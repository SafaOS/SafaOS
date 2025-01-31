const std_c = @import("std-c");
const std = @import("std");

const print = std_c.print;
const File = std_c.stdio.File;
const allocator = std_c.heap.c_allocator;

pub const panic = std_c.panic;

pub fn main() !void {
    const args = std_c.sys.args();
    var data: []u8 = undefined;

    if (args.count() > 1) {
        const filename = args.nth(1).?;

        const file = try File.open(filename, .{ .read = true });
        defer file.close();

        data = try file.reader().readAllAlloc(allocator, std.math.maxInt(usize));
    } else {
        const stdin = std_c.stdin.reader();
        const stdin_data = try stdin.readUntilDelimiterOrEofAlloc(std_c.heap.c_allocator, '\n', std.math.maxInt(usize));
        data = stdin_data.?;
    }

    print("{s}\n", .{data});
}

comptime {
    _ = std_c;
}
