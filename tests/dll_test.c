#include <stdio.h>
#include <string.h>
#include <windows.h>

typedef struct {
    size_t (*read_buffer)(void* ctx, char* buf, size_t buf_len);
    void (*send_input)(void* ctx, const unsigned char* text, size_t len, int raw);
    size_t (*tab_count)(void* ctx);
    size_t (*active_tab)(void* ctx);
    void (*switch_tab)(void* ctx, size_t index);
    void (*new_tab)(void* ctx);
    void (*close_tab)(void* ctx, size_t index);
    void (*focus)(void* ctx);
    size_t (*hwnd_fn)(void* ctx);
    size_t (*tab_title)(void* ctx, size_t index, char* buf, size_t buf_len);
    size_t (*tab_working_dir)(void* ctx, size_t index, char* buf, size_t buf_len);
    int (*tab_has_selection)(void* ctx, size_t index);
    void* ctx;
} TerminalProviderVTable;

// Mock implementations
static size_t mock_read_buffer(void* ctx, char* buf, size_t buf_len) {
    (void)ctx;
    const char* text = "PS C:\\Users\\yuuji> ";
    size_t len = strlen(text);
    if (len > buf_len) len = buf_len;
    memcpy(buf, text, len);
    return len;
}
static void mock_send_input(void* ctx, const unsigned char* text, size_t len, int raw) {
    (void)ctx; (void)text; (void)len; (void)raw;
    printf("[mock] send_input called (len=%zu, raw=%d)\n", len, raw);
}
static size_t mock_tab_count(void* ctx) { (void)ctx; return 1; }
static size_t mock_active_tab(void* ctx) { (void)ctx; return 0; }
static void mock_switch_tab(void* ctx, size_t i) { (void)ctx; (void)i; }
static void mock_new_tab(void* ctx) { (void)ctx; }
static void mock_close_tab(void* ctx, size_t i) { (void)ctx; (void)i; }
static void mock_focus(void* ctx) { (void)ctx; }
static size_t mock_hwnd(void* ctx) { (void)ctx; return 0x12345; }
static size_t mock_tab_title(void* ctx, size_t i, char* buf, size_t len) {
    (void)ctx; (void)i;
    const char* t = "MockTab";
    size_t n = strlen(t); if (n > len) n = len;
    memcpy(buf, t, n); return n;
}
static size_t mock_tab_working_dir(void* ctx, size_t i, char* buf, size_t len) {
    (void)ctx; (void)i;
    const char* t = "C:\\Users\\yuuji";
    size_t n = strlen(t); if (n > len) n = len;
    memcpy(buf, t, n); return n;
}
static int mock_tab_has_selection(void* ctx, size_t i) { (void)ctx; (void)i; return 0; }

// DLL function types
typedef void* (*fn_create)(const char*, const TerminalProviderVTable*);
typedef int (*fn_start)(void*);
typedef void (*fn_stop)(void*);
typedef void (*fn_destroy)(void*);

int main(void) {
    HMODULE dll = LoadLibraryA("control_plane_server.dll");
    if (!dll) {
        printf("FAIL: LoadLibrary failed (err=%lu)\n", GetLastError());
        return 1;
    }
    printf("[test] DLL loaded\n");

    fn_create create = (fn_create)GetProcAddress(dll, "cp_server_create");
    fn_start start = (fn_start)GetProcAddress(dll, "cp_server_start");
    fn_stop stop = (fn_stop)GetProcAddress(dll, "cp_server_stop");
    fn_destroy destroy = (fn_destroy)GetProcAddress(dll, "cp_server_destroy");

    if (!create || !start || !stop || !destroy) {
        printf("FAIL: GetProcAddress failed\n");
        FreeLibrary(dll);
        return 1;
    }
    printf("[test] All 4 exports found\n");

    int dummy = 0;
    TerminalProviderVTable vtable = {
        .read_buffer = mock_read_buffer,
        .send_input = mock_send_input,
        .tab_count = mock_tab_count,
        .active_tab = mock_active_tab,
        .switch_tab = mock_switch_tab,
        .new_tab = mock_new_tab,
        .close_tab = mock_close_tab,
        .focus = mock_focus,
        .hwnd_fn = mock_hwnd,
        .tab_title = mock_tab_title,
        .tab_working_dir = mock_tab_working_dir,
        .tab_has_selection = mock_tab_has_selection,
        .ctx = &dummy,
    };

    printf("[test] Creating server...\n");
    void* server = create("dll-test", &vtable);
    if (!server) {
        printf("FAIL: cp_server_create returned NULL\n");
        FreeLibrary(dll);
        return 1;
    }
    printf("[test] Server created\n");

    printf("[test] Starting server...\n");
    int rc = start(server);
    if (rc != 0) {
        printf("FAIL: cp_server_start returned %d\n", rc);
        destroy(server);
        FreeLibrary(dll);
        return 1;
    }
    printf("[test] Server started\n");

    // Wait for pipe to be ready
    Sleep(500);

    // Connect to pipe and send PING
    DWORD pid = GetCurrentProcessId();
    char pipe_path[256];
    snprintf(pipe_path, sizeof(pipe_path),
        "\\\\.\\pipe\\windows-terminal-winui3-dll-test-%lu-rs", (unsigned long)pid);
    printf("[test] Connecting to %s\n", pipe_path);

    HANDLE pipe = CreateFileA(pipe_path, GENERIC_READ | GENERIC_WRITE,
        0, NULL, OPEN_EXISTING, 0, NULL);
    if (pipe == INVALID_HANDLE_VALUE) {
        printf("FAIL: CreateFile failed (err=%lu)\n", GetLastError());
        stop(server);
        destroy(server);
        FreeLibrary(dll);
        return 1;
    }

    // Send PING
    const char* msg = "PING";
    DWORD written = 0;
    WriteFile(pipe, msg, (DWORD)strlen(msg), &written, NULL);
    FlushFileBuffers(pipe);

    // Read response
    char resp[512] = {0};
    DWORD read_bytes = 0;
    ReadFile(pipe, resp, sizeof(resp) - 1, &read_bytes, NULL);
    CloseHandle(pipe);

    printf("[test] PING response: %s", resp);

    // Verify PONG
    if (strncmp(resp, "PONG|dll-test|", 14) == 0) {
        printf("[test] PASS: PONG received with correct session name\n");
    } else {
        printf("FAIL: unexpected response\n");
        stop(server);
        destroy(server);
        FreeLibrary(dll);
        return 1;
    }

    // Test AGENT_STATUS
    Sleep(200);
    pipe = CreateFileA(pipe_path, GENERIC_READ | GENERIC_WRITE,
        0, NULL, OPEN_EXISTING, 0, NULL);
    if (pipe != INVALID_HANDLE_VALUE) {
        msg = "AGENT_STATUS";
        WriteFile(pipe, msg, (DWORD)strlen(msg), &written, NULL);
        FlushFileBuffers(pipe);
        memset(resp, 0, sizeof(resp));
        ReadFile(pipe, resp, sizeof(resp) - 1, &read_bytes, NULL);
        CloseHandle(pipe);
        printf("[test] AGENT_STATUS response: %s", resp);
        if (strncmp(resp, "AGENT_STATUS|dll-test|", 21) == 0) {
            printf("[test] PASS: AGENT_STATUS received\n");
        }
    }

    printf("[test] Stopping...\n");
    stop(server);
    destroy(server);
    FreeLibrary(dll);
    printf("[test] PASS: Full lifecycle completed\n");
    return 0;
}
