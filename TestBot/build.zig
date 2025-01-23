const std = @import("std");
// Although this function looks imperative, note that its job is to
// declaratively construct a build graph that will be executed by an external
// runner.
pub fn build(b: *std.Build) void {
    // Standard target options allows the person running `zig build` to choose
    // what target to build for. Here we do not override the defaults, which
    // means any target is allowed, and the default is native. Other options
    // for restricting supported target set are available.
    const target = b.standardTargetOptions(.{ .default_target = .{ .abi = .none, .os_tag = .freestanding, .cpu_features_sub = std.Target.x86.featureSet(&[_]std.Target.x86.Feature{ .avx, .avx2, .sse, .sse2 }) } });
    // Standard optimization options allow the person running `zig build` to select
    // between Debug, ReleaseSafe, ReleaseFast, and ReleaseSmall. Here we do not
    // set a preferred release mode, allowing the user to decide how to optimize.
    const optimize = b.standardOptimizeOption(.{ .preferred_optimize_mode = .ReleaseSafe });
    const libc = b.addModule("libc", .{
        .root_source_file = b.path("../libc/src/root.zig"),
    });

    const std_c = b.addModule("std-c", .{
        .root_source_file = b.path("../std-c/src/root.zig"),
    });

    std_c.addImport("libc", libc);
    const exe = b.addExecutable(.{
        .name = "TestBot",
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
        .linkage = .static,
    });

    exe.root_module.addImport("std-c", std_c);
    // This declares intent for the executable to be installed into the
    // standard location when the user invokes the "install" step (the default
    // step when running `zig build`).
    b.installArtifact(exe);

    const check_step = b.step("check", "Check the code");
    check_step.dependOn(&exe.step);
}
