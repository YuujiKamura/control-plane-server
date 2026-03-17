const std = @import("std");

// C ABI types matching ffi.rs
const TerminalProviderVTable = extern struct {
    read_buffer: *const fn (ctx: *anyopaque, buf: [*]u8, buf_len: usize) callconv(.C) usize,
    send_input: *const fn (ctx: *anyopaque, text: [*]const u8, len: usize, raw: bool) callconv(.C) void,
    tab_count: *const fn (ctx: *anyopaque) callconv(.C) usize,
    active_tab: *const fn (ctx: *anyopaque) callconv(.C) usize,
    switch_tab: *const fn (ctx: *anyopaque, index: usize) callconv(.C) void,
    new_tab: *const fn (ctx: *anyopaque) callconv(.C) void,
    close_tab: *const fn (ctx: *anyopaque, index: usize) callconv(.C) void,
    focus: *const fn (ctx: *anyopaque) callconv(.C) void,
    hwnd: *const fn (ctx: *anyopaque) callconv(.C) usize,
    tab_title: *const fn (ctx: *anyopaque, index: usize, buf: [*]u8, buf_len: usize) callconv(.C) usize,
    tab_working_dir: *const fn (ctx: *anyopaque, index: usize, buf: [*]u8, buf_len: usize) callconv(.C) usize,
    tab_has_selection: *const fn (ctx: *anyopaque, index: usize) callconv(.C) bool,
    ctx: *anyopaque,
};

// Import DLL functions
extern "control_plane_server" fn cp_server_create(session_name: [*:0]const u8, provider: *const TerminalProviderVTable) callconv(.C) ?*anyopaque;
extern "control_plane_server" fn cp_server_start(server: *anyopaque) callconv(.C) i32;
extern "control_plane_server" fn cp_server_stop(server: *anyopaque) callconv(.C) void;
extern "control_plane_server" fn cp_server_destroy(server: *anyopaque) callconv(.C) void;

// Mock implementations
fn mockReadBuffer(_: *anyopaque, buf: [*]u8, buf_len: usize) callconv(.C) usize {
    const text = "PS C:\\Users\\yuuji> ";
    const len = @min(text.len, buf_len);
    @memcpy(buf[0..len], text[0..len]);
    return len;
}

fn mockSendInput(_: *anyopaque, _: [*]const u8, _: usize, _: bool) callconv(.C) void {}
fn mockTabCount(_: *anyopaque) callconv(.C) usize { return 1; }
fn mockActiveTab(_: *anyopaque) callconv(.C) usize { return 0; }
fn mockSwitchTab(_: *anyopaque, _: usize) callconv(.C) void {}
fn mockNewTab(_: *anyopaque) callconv(.C) void {}
fn mockCloseTab(_: *anyopaque, _: usize) callconv(.C) void {}
fn mockFocus(_: *anyopaque) callconv(.C) void {}
fn mockHwnd(_: *anyopaque) callconv(.C) usize { return 0x12345; }

fn mockTabTitle(_: *anyopaque, _: usize, buf: [*]u8, buf_len: usize) callconv(.C) usize {
    const text = "Mock Tab";
    const len = @min(text.len, buf_len);
    @memcpy(buf[0..len], text[0..len]);
    return len;
}

fn mockTabWorkingDir(_: *anyopaque, _: usize, buf: [*]u8, buf_len: usize) callconv(.C) usize {
    const text = "C:\\Users\\yuuji";
    const len = @min(text.len, buf_len);
    @memcpy(buf[0..len], text[0..len]);
    return len;
}

fn mockTabHasSelection(_: *anyopaque, _: usize) callconv(.C) bool { return false; }

pub fn main() !void {
    const stdout = std.io.getStdOut().writer();

    // Dummy context (not used by mocks)
    var dummy: u8 = 0;
    const ctx: *anyopaque = @ptrCast(&dummy);

    var vtable = TerminalProviderVTable{
        .read_buffer = &mockReadBuffer,
        .send_input = &mockSendInput,
        .tab_count = &mockTabCount,
        .active_tab = &mockActiveTab,
        .switch_tab = &mockSwitchTab,
        .new_tab = &mockNewTab,
        .close_tab = &mockCloseTab,
        .focus = &mockFocus,
        .hwnd = &mockHwnd,
        .tab_title = &mockTabTitle,
        .tab_working_dir = &mockTabWorkingDir,
        .tab_has_selection = &mockTabHasSelection,
        .ctx = ctx,
    };

    try stdout.print("[zig-test] Creating server...\n", .{});
    const server = cp_server_create("zig-test", &vtable);
    if (server == null) {
        try stdout.print("[zig-test] FAIL: cp_server_create returned null\n", .{});
        return error.ServerCreateFailed;
    }
    try stdout.print("[zig-test] Server created: {*}\n", .{server.?});

    try stdout.print("[zig-test] Starting server...\n", .{});
    const rc = cp_server_start(server.?);
    if (rc != 0) {
        try stdout.print("[zig-test] FAIL: cp_server_start returned {}\n", .{rc});
        cp_server_destroy(server.?);
        return error.ServerStartFailed;
    }
    try stdout.print("[zig-test] Server started (rc={})\n", .{rc});

    // Give pipe server time to start
    std.time.sleep(500 * std.time.ns_per_ms);

    // Connect to pipe and send PING
    const pid = std.os.windows.GetCurrentProcess();
    _ = pid;
    try stdout.print("[zig-test] Connecting to pipe...\n", .{});

    // Use agent-ctl to test (simpler than raw pipe from Zig)
    // The pipe name will be: \\.\pipe\windows-terminal-winui3-zig-test-{pid}-rs
    try stdout.print("[zig-test] Server is running. Use agent-ctl to test.\n", .{});

    // Keep alive for 5 seconds for manual testing
    std.time.sleep(5 * std.time.ns_per_s);

    try stdout.print("[zig-test] Stopping server...\n", .{});
    cp_server_stop(server.?);
    try stdout.print("[zig-test] Destroying server...\n", .{});
    cp_server_destroy(server.?);
    try stdout.print("[zig-test] PASS: Full lifecycle completed\n", .{});
}
