use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::Arc;
use crate::{TerminalProvider, TabInfo};
use crate::server::ControlPlaneServer;

#[repr(C)]
pub struct TerminalProviderVTable {
    pub read_buffer: extern "C" fn(ctx: *mut c_void, buf: *mut c_char, buf_len: usize) -> usize,
    pub send_input: extern "C" fn(ctx: *mut c_void, text: *const u8, len: usize, raw: bool),
    pub tab_count: extern "C" fn(ctx: *mut c_void) -> usize,
    pub active_tab: extern "C" fn(ctx: *mut c_void) -> usize,
    pub switch_tab: extern "C" fn(ctx: *mut c_void, index: usize),
    pub new_tab: extern "C" fn(ctx: *mut c_void),
    pub close_tab: extern "C" fn(ctx: *mut c_void, index: usize),
    pub focus: extern "C" fn(ctx: *mut c_void),
    pub hwnd: extern "C" fn(ctx: *mut c_void) -> usize,
    pub tab_title: extern "C" fn(ctx: *mut c_void, index: usize, buf: *mut c_char, buf_len: usize) -> usize,
    pub tab_working_dir: extern "C" fn(ctx: *mut c_void, index: usize, buf: *mut c_char, buf_len: usize) -> usize,
    pub tab_has_selection: extern "C" fn(ctx: *mut c_void, index: usize) -> bool,
    pub ctx: *mut c_void,
}

struct FfiBridge {
    vtable: TerminalProviderVTable,
}

// TerminalProviderVTable is expected to be thread-safe if the underlying terminal is.
unsafe impl Send for FfiBridge {}
unsafe impl Sync for FfiBridge {}

impl TerminalProvider for FfiBridge {
    fn read_buffer(&self) -> String {
        let mut buf = vec![0u8; 65536];
        let n = (self.vtable.read_buffer)(self.vtable.ctx, buf.as_mut_ptr() as *mut c_char, buf.len());
        let n = n.min(buf.len());
        String::from_utf8_lossy(&buf[..n]).to_string()
    }

    fn send_input(&self, text: &[u8], raw: bool) {
        (self.vtable.send_input)(self.vtable.ctx, text.as_ptr(), text.len(), raw);
    }

    fn tab_count(&self) -> usize {
        (self.vtable.tab_count)(self.vtable.ctx)
    }

    fn active_tab(&self) -> usize {
        (self.vtable.active_tab)(self.vtable.ctx)
    }

    fn tab_info(&self, index: usize) -> Option<TabInfo> {
        let mut title_buf = vec![0u8; 1024];
        let n_title = (self.vtable.tab_title)(self.vtable.ctx, index, title_buf.as_mut_ptr() as *mut c_char, title_buf.len());
        let n_title = n_title.min(title_buf.len());
        if n_title == 0 && index >= self.tab_count() {
            return None;
        }
        let title = String::from_utf8_lossy(&title_buf[..n_title]).to_string();

        let mut dir_buf = vec![0u8; 1024];
        let n_dir = (self.vtable.tab_working_dir)(self.vtable.ctx, index, dir_buf.as_mut_ptr() as *mut c_char, dir_buf.len());
        let n_dir = n_dir.min(dir_buf.len());
        let working_directory = String::from_utf8_lossy(&dir_buf[..n_dir]).to_string();

        let has_selection = (self.vtable.tab_has_selection)(self.vtable.ctx, index);

        Some(TabInfo {
            title,
            working_directory,
            has_selection,
        })
    }

    fn switch_tab(&self, index: usize) {
        (self.vtable.switch_tab)(self.vtable.ctx, index);
    }

    fn new_tab(&self) {
        (self.vtable.new_tab)(self.vtable.ctx);
    }

    fn close_tab(&self, index: usize) {
        (self.vtable.close_tab)(self.vtable.ctx, index);
    }

    fn focus(&self) {
        (self.vtable.focus)(self.vtable.ctx);
    }

    fn hwnd(&self) -> usize {
        (self.vtable.hwnd)(self.vtable.ctx)
    }
}

#[no_mangle]
pub extern "C" fn cp_server_create(session_name: *const c_char, provider_vtable: *const TerminalProviderVTable) -> *mut c_void {
    cp_server_create_with_prefix(session_name, ptr::null(), provider_vtable)
}

/// Create a server with a custom pipe name prefix.
/// If pipe_prefix is null, defaults to "windows-terminal-winui3".
#[no_mangle]
pub extern "C" fn cp_server_create_with_prefix(session_name: *const c_char, pipe_prefix: *const c_char, provider_vtable: *const TerminalProviderVTable) -> *mut c_void {
    if session_name.is_null() || provider_vtable.is_null() {
        return ptr::null_mut();
    }

    let name = unsafe { CStr::from_ptr(session_name) }.to_string_lossy().into_owned();
    let prefix = if pipe_prefix.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(pipe_prefix) }.to_string_lossy().into_owned())
    };
    let vtable = unsafe { ptr::read(provider_vtable) };

    let bridge = Arc::new(FfiBridge { vtable });
    match ControlPlaneServer::new(bridge, name, prefix) {
        Ok(server) => Box::into_raw(Box::new(server)) as *mut c_void,
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn cp_server_start(server: *mut c_void) -> i32 {
    if server.is_null() {
        return -1;
    }
    let server = unsafe { &*(server as *mut ControlPlaneServer) };
    match server.start() {
        Ok(_) => 0,
        Err(_) => -2,
    }
}

#[no_mangle]
pub extern "C" fn cp_server_stop(server: *mut c_void) {
    if !server.is_null() {
        let server = unsafe { &*(server as *mut ControlPlaneServer) };
        server.stop();
    }
}

#[no_mangle]
pub extern "C" fn cp_server_destroy(server: *mut c_void) {
    if !server.is_null() {
        unsafe {
            let _ = Box::from_raw(server as *mut ControlPlaneServer);
        }
    }
}
