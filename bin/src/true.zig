// usize instead of bool because zig treats bools as u8s
// and in SafaOS main is expected to return usize
export fn main() usize {
    return 1;
}

comptime {
    _ = @import("std-c");
}
