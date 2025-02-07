pub const panic = @import("std-c").panic;

export fn main() bool {
    return true;
}

comptime {
    _ = @import("std-c");
}
