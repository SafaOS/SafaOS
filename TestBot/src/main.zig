const std_c = @import("std-c");
const std = @import("std");

const File = std_c.stdio.FILE;
const Slice = std_c.sys.raw.Slice;
const spawn = std_c.sys.utils.zpspwan;
const wait = std_c.syscalls.wait;

const allocator = std_c.heap.c_allocator;
var serial: *File = undefined;

var last_output: ?Output = null;
var expected_output: ?Output = null;
/// meminfo at the start of the tests
/// used to check if there is memory leaks
var meminfo_output: Output = undefined;

/// TODO: make this a union with different errors, containing payloads
var extra_info: ?[:0]const u8 = null;

pub fn print(fmt: [:0]const u8, args: anytype) void {
    serial.writer().writeFmt(fmt, args) catch {};
}

const NativeError = std_c.sys.errno.Error;
const ExtraError = error{ UnexpectedError, UnexpectedStatus, UnexpectedStdout };

const Error = NativeError || ExtraError;
const Output = struct {
    stdout: []const u8,
    status: u64,
    pub fn expect(self: *const Output, expected: ?[]const u8, status: ?u64) !void {
        last_output = self.*;
        expected_output = .{ .stdout = expected orelse "", .status = status orelse 0 };

        if (status) |stat| if (self.status != stat) return error.UnexpectedStatus;
        if (expected) |exp| {
            if (self.stdout.len != exp.len) {
                extra_info = "stdout length mismatch";
                return error.UnexpectedStdout;
            }

            for (self.stdout, 0..) |byte, i| {
                if (byte != exp[i]) {
                    extra_info = "stdout byte mismatch";
                    return error.UnexpectedStdout;
                }
            }
        }
    }

    pub fn eql(self: *const Output, other: *const Output) bool {
        if (self.status != other.status) return false;
        if (self.stdout.len != other.stdout.len) return false;
        for (self.stdout, 0..) |byte, i| {
            if (byte != other.stdout[i]) return false;
        }
        return true;
    }

    pub fn debug(self: *const Output) void {
        print(
            \\stdout (%l):
            \\%.*s
            \\status: %l
            \\
        , .{ self.stdout.len, self.stdout.len, self.stdout.ptr, self.status });
    }

    pub fn uninit(self: Output) void {
        allocator.free(self.stdout);
    }
};

fn make_args(args: anytype) []const Slice(u8) {
    comptime var i: usize = 0;

    const info = @typeInfo(@TypeOf(args));
    const fields = info.Struct.fields;
    var args_array: [fields.len]Slice(u8) = undefined;

    inline for (fields) |field| {
        const value = @field(args, field.name);

        args_array[i] = Slice(u8).from(value);
        i += 1;
    }
    return &args_array;
}
/// executes a binary with arguments and returns the output worte to fd 1
fn test_binary(comptime path: []const u8, args: []const Slice(u8)) !Output {
    const test_log = try File.open("ram:/test.txt", .{ .write = true, .read = true });
    defer test_log.close();

    const pid = try spawn(path, args, "[TestCase]: " ++ path);
    const status = wait(pid);

    const buffer = try test_log.reader().readUntilEOF();
    return .{ .stdout = buffer, .status = status };
}

fn meminfo() !Output {
    const output = try test_binary("sys:/bin/meminfo", &[_]Slice(u8){Slice(u8).from("-k")});
    try output.expect(null, 0);
    return output;
}

fn mkdir(dir: []const u8) !void {
    const output = try test_binary(
        "sys:/bin/mkdir",
        make_args(.{ "mkdir", dir }),
    );
    try output.expect(null, 0);
    output.uninit();
}

fn touch(file: []const u8) !void {
    const output = try test_binary("sys:/bin/touch", make_args(.{ "touch", file }));
    try output.expect(null, 0);
    output.uninit();
}

fn write(file: []const u8, data: []const u8) !void {
    const output = try test_binary("sys:/bin/write", make_args(.{ "write", file, data }));
    try output.expect(null, 0);
    output.uninit();
}

fn cat(file: []const u8) !Output {
    const output = try test_binary("sys:/bin/cat", make_args(.{ "cat", file }));
    return output;
}

fn get_cwd() ![]u8 {
    const buffer = try allocator.alloc(u8, 1024);
    const len = try std_c.sys.io.zgetcwd(&buffer);

    return buffer[0..len];
}

fn chdir(dir: []const u8) !void {
    try std_c.sys.io.zchdir(dir);
}

fn ls() !Output {
    const output = try test_binary("sys:/bin/ls", &[_]Slice(u8){Slice(u8).from("--raw")});
    return output;
}

pub fn memory_info_capture() Error!void {
    const output = try meminfo();
    meminfo_output = output;
}

pub fn echo_test() Error!void {
    const output = try test_binary("sys:/bin/echo", make_args(.{ "echo", "test data" }));
    try output.expect("test data\n", 0);
    output.uninit();
}

pub fn mkdir_test() Error!void {
    try mkdir("test");
}

pub fn touch_test() Error!void {
    try touch("test/test_file");
}

pub fn write_test() Error!void {
    try write("test/test_file", "test data");
}

pub fn cat_test() Error!void {
    const output = try cat("test/test_file");
    try output.expect("test data\n", 0);
    output.uninit();
}

pub fn cd_test() Error!void {
    try chdir("test");
    const output = try cat("test_file");
    try output.expect("test data\n", 0);
    output.uninit();
    try chdir("..");
}

pub fn ls_test() Error!void {
    try chdir("test");
    const output = try ls();
    try output.expect(
        \\..
        \\test_file
        \\
    , 0);
    output.uninit();
}

pub fn memory_info_test() Error!void {
    const output = try meminfo();
    if (!meminfo_output.eql(&output)) {
        print("\x1b[31m[TestBot]: ", .{});
        print(
            \\possible memory leak detected
            \\expected:
            \\%.*s
            \\actual:
            \\%.*s
            \\
        , .{ meminfo_output.stdout.len - 1, meminfo_output.stdout.ptr, output.stdout.len - 1, output.stdout.ptr });
        print("\x1b[0m", .{});
    } else print("\x1b[36m[TestBot]\x1b[0m: memory has been reported to be %.*s since the start of the TestBot, no possible leaks detected\n", .{ output.stdout.len - 1, output.stdout.ptr });
    output.uninit();
}

fn run_test(comptime name: []const u8, func: fn () Error!void) Error!void {
    print("\x1b[36m[TEST]\x1b[0m running: " ++ name ++ "\n", .{});

    func() catch |err| {
        const err_name = @errorName(err);
        print("\x1b[31m[FAILED]: %.*s\n", .{ err_name.len, err_name.ptr });
        if (last_output) |output| {
            output.debug();
        }

        if (expected_output) |exp| {
            print("Expected (may not be accurate, see text case for more info):\n", .{});
            exp.debug();
        }
        if (extra_info) |info| {
            print("Extra info: %s\n", .{info.ptr});
        }

        print("\x1b[0m", .{});
        return err;
    };

    print("\x1b[32m[OK]\x1b[0m\n", .{});
}

const TestCase = struct { name: []const u8, func: fn () Error!void };

fn get_tests() []const TestCase {
    const info = @typeInfo(@This());

    // tests are ran in the order they are defined in
    comptime var tests: []const TestCase = &[_]TestCase{};
    inline for (info.Struct.decls) |decl| {
        const func = @field(@This(), decl.name);
        if (@TypeOf(func) == fn () Error!void)
            tests = tests ++ &[_]TestCase{.{ .name = decl.name, .func = func }};
    }

    return tests;
}

fn run_tests(comptime tests: []const TestCase) Error!void {
    inline for (tests) |test_case| {
        try run_test(test_case.name, test_case.func);
    }

    print("\x1b[36m[TestBot]\x1b[0m: \x1b[32m[PASSED]\x1b[0m\n", .{});
}
pub fn main() !void {
    // fd 0
    serial = try File.open("dev:/ss", .{ .write = true, .read = true });
    defer serial.close();

    const tests = get_tests();
    print("\x1b[36m[TEST]\x1b[0m: TestBot running %l tests ...\n", .{tests.len});
    try run_tests(tests);
}

comptime {
    _ = std_c;
}
