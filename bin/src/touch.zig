const std_c = @import("std-c");
const File = std_c.stdio.File;
const print = std_c.print;
const Error = std_c.Error;

pub fn main() Error!void {
    var args = std_c.sys.args();

    if (args.count() < 2) {
        print("expected at least the file name to touch!\n", .{});
        return error.NotEnoughArguments;
    }

    const filename = args.nth(1).?;
    const file = File.open(filename, .{ .read = true }) catch |err|
        switch (err) {
        error.NoSuchAFileOrDirectory => try File.open(filename, .{ .write = true }),
        else => {
            print("failed to open file {s}, err {s}\n", .{ filename, @errorName(err) });
            return err;
        },
    };

    file.close();
}

comptime {
    _ = std_c;
}
