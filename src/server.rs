use std::sync::{Arc, Mutex};
use std::thread;


use windows::core::{PCWSTR, HSTRING};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows::Win32::Storage::FileSystem::{
    ReadFile, WriteFile, FlushFileBuffers, FILE_FLAG_OVERLAPPED,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
const PIPE_ACCESS_DUPLEX: u32 = 3;
const SDDL_REVISION_1: u32 = 1;

use windows::Win32::System::Threading::{
    CreateEventW, WaitForSingleObject, GetCurrentProcessId,
};
use windows::Win32::System::IO::{GetOverlappedResult, CancelIoEx, OVERLAPPED};
use windows::Win32::Security::{
    SECURITY_ATTRIBUTES, PSECURITY_DESCRIPTOR,
};
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;

use crate::error::Result;
use crate::protocol::{Request, TabTarget, escape_field};

use crate::session::{SessionManager, sanitize_session_name};
use crate::tab_id::TabIdManager;
use crate::TerminalProvider;
use crate::utils::{slice_last_lines, infer_prompt};

const MAX_READ_SIZE: u32 = 65536;

pub struct ControlPlaneServer {
    provider: Arc<dyn TerminalProvider>,
    tab_id_manager: Arc<Mutex<TabIdManager>>,
    session_manager: Arc<SessionManager>,
    stop: Arc<Mutex<bool>>,
}

impl ControlPlaneServer {
    pub fn new(provider: Arc<dyn TerminalProvider>, session_name: String, pipe_prefix: Option<String>) -> Result<Self> {
        let pid = unsafe { GetCurrentProcessId() };
        let safe_session_name = sanitize_session_name(&session_name);
        let prefix = pipe_prefix.as_deref().unwrap_or("windows-terminal-winui3");
        let pipe_name = format!("{}-{}-{}", prefix, safe_session_name, pid);
        let pipe_path = format!(r"\\.\pipe\{}", pipe_name);

        let session_manager = Arc::new(SessionManager::new(session_name, pipe_path, pipe_prefix.as_deref())?);

        Ok(Self {
            provider,
            tab_id_manager: Arc::new(Mutex::new(TabIdManager::new())),
            session_manager,
            stop: Arc::new(Mutex::new(false)),
        })
    }

    pub fn start(&self) -> Result<()> {
        let hwnd = self.provider.hwnd();
        self.session_manager.write_file(hwnd)?;

        let provider = self.provider.clone();
        let tab_id_manager = self.tab_id_manager.clone();
        let pipe_path = self.session_manager.pipe_path.clone();
        let session_name = self.session_manager.session_name.clone();
        let pid = self.session_manager.pid;
        let stop = self.stop.clone();

        thread::spawn(move || {
            if let Err(e) = server_thread_main(provider, tab_id_manager, pipe_path, session_name, pid, stop) {
                eprintln!("ControlPlaneServer error: {:?}", e);
            }
        });

        Ok(())
    }

    pub fn stop(&self) {
        let mut stop = self.stop.lock().unwrap();
        *stop = true;
        self.session_manager.remove_file();
    }
}

fn server_thread_main(
    provider: Arc<dyn TerminalProvider>,
    tab_id_manager: Arc<Mutex<TabIdManager>>,
    pipe_path: String,
    session_name: String,
    pid: u32,
    stop: Arc<Mutex<bool>>,
) -> Result<()> {
    let pipe_path_w = HSTRING::from(&pipe_path);

    // Security Descriptor: "D:(A;;GA;;;OW)" (Owner Generic All)
    let mut sa = SECURITY_ATTRIBUTES::default();
    sa.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
    sa.bInheritHandle = false.into();

    let mut psd: PSECURITY_DESCRIPTOR = PSECURITY_DESCRIPTOR::default();
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from("D:(A;;GA;;;OW)"),
            SDDL_REVISION_1,
            &mut psd,
            None
        ).is_ok() {
            sa.lpSecurityDescriptor = psd.0;
        }
    }

    while !*stop.lock().unwrap() {
        let pipe = unsafe {
            CreateNamedPipeW(
                PCWSTR(pipe_path_w.as_ptr()),
                windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED.0),
                windows::Win32::System::Pipes::NAMED_PIPE_MODE(PIPE_TYPE_BYTE.0 | PIPE_WAIT.0),
                1,
                MAX_READ_SIZE,
                MAX_READ_SIZE,
                0,
                Some(&sa),
            )
        };

        if pipe == INVALID_HANDLE_VALUE {
            break;
        }

        let mut ov = OVERLAPPED::default();
        ov.hEvent = unsafe { CreateEventW(None, true, false, None)? };

        let mut connected = false;
        unsafe {
            let res = ConnectNamedPipe(pipe, Some(&mut ov));
            if res.is_ok() {
                connected = true;
            } else {
                let err = GetLastError();
                if err == windows::Win32::Foundation::ERROR_PIPE_CONNECTED {
                    connected = true;
                } else if err == windows::Win32::Foundation::ERROR_IO_PENDING {
                    while !*stop.lock().unwrap() {
                        let wait = WaitForSingleObject(ov.hEvent, 1000);
                        if wait == windows::Win32::Foundation::WAIT_OBJECT_0 {
                            let mut dummy = 0;
                            connected = GetOverlappedResult(pipe, &ov, &mut dummy, false).is_ok();
                            break;
                        }
                    }
                    if *stop.lock().unwrap() && !connected {
                        let _ = CancelIoEx(pipe, Some(&ov));
                    }
                }
            }
        }

        if connected && !*stop.lock().unwrap() {
            handle_client(pipe, &provider, &tab_id_manager, &session_name, pid);
        }

        unsafe {
            let _ = DisconnectNamedPipe(pipe);
            let _ = CloseHandle(ov.hEvent);
            let _ = CloseHandle(pipe);
        }
    }

    unsafe {
        if !psd.0.is_null() {
            let _ = windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(psd.0 as _));
        }
    }

    Ok(())
}

fn handle_client(
    pipe: HANDLE,
    provider: &Arc<dyn TerminalProvider>,
    tab_id_manager: &Arc<Mutex<TabIdManager>>,
    session_name: &str,
    pid: u32,
) {
    let mut buffer = vec![0u8; MAX_READ_SIZE as usize];
    let mut read = 0;
    let mut ov = OVERLAPPED::default();
    ov.hEvent = unsafe { CreateEventW(None, true, false, None).unwrap() };

    let mut read_ok = false;
    unsafe {
        let res = ReadFile(pipe, Some(buffer.as_mut_slice()), Some(&mut read), Some(&mut ov));
        if res.is_ok() {
            read_ok = true;
        } else if GetLastError() == windows::Win32::Foundation::ERROR_IO_PENDING {
            let wait = WaitForSingleObject(ov.hEvent, 10000);
            if wait == windows::Win32::Foundation::WAIT_OBJECT_0 {
                read_ok = GetOverlappedResult(pipe, &ov, &mut read, false).is_ok();
            } else {
                let _ = CancelIoEx(pipe, Some(&ov));
            }
        }
    }

    if read_ok && read > 0 {
        let request_str = String::from_utf8_lossy(&buffer[..read as usize]);
        let trimmed = request_str.trim();
        if !trimmed.is_empty() {
            let response = build_response(trimmed, provider, tab_id_manager, session_name, pid);
            let mut written = 0;
            unsafe {
                let _ = WriteFile(pipe, Some(response.as_bytes()), Some(&mut written), None);
                let _ = FlushFileBuffers(pipe);
            }
        }
    }

    unsafe {
        let _ = CloseHandle(ov.hEvent);
    }
}

/// Resolve a `TabTarget` to a positional index.  Returns `Err(response)`
/// if the target cannot be resolved and we should send an error back.
fn resolve_tab(
    target: &TabTarget,
    tab_id_manager: &Mutex<TabIdManager>,
    _provider: &Arc<dyn TerminalProvider>,
    session_name: &str,
) -> std::result::Result<Option<usize>, String> {
    match target {
        TabTarget::None => Ok(None),
        TabTarget::Index(idx) => Ok(Some(*idx)),
        TabTarget::Id(id) => {
            let mgr = tab_id_manager.lock().unwrap();
            match mgr.resolve(id) {
                Some(idx) => Ok(Some(idx)),
                None => Err(format!("ERR|{}|unknown-tab-id|{}\n", session_name, id)),
            }
        }
    }
}

/// Resolve a `TabTarget`, defaulting to the active tab when `None`.
fn resolve_tab_or_active(
    target: &TabTarget,
    tab_id_manager: &Mutex<TabIdManager>,
    provider: &Arc<dyn TerminalProvider>,
    session_name: &str,
) -> std::result::Result<usize, String> {
    match resolve_tab(target, tab_id_manager, provider, session_name)? {
        Some(idx) => Ok(idx),
        None => Ok(provider.active_tab()),
    }
}

fn build_response(
    request_str: &str,
    provider: &Arc<dyn TerminalProvider>,
    tab_id_manager: &Arc<Mutex<TabIdManager>>,
    session_name: &str,
    pid: u32,
) -> String {
    let request = match Request::parse(request_str) {
        Ok(r) => r,
        Err(e) => return format!("ERR|{}|{:?}\n", session_name, e),
    };

    match request {
        Request::Ping => {
            format!("PONG|{}|{}|0x{:X}\n", session_name, pid, provider.hwnd())
        }
        Request::State(tab) => {
            let idx = match resolve_tab_or_active(&tab, tab_id_manager, provider, session_name) {
                Ok(i) => i,
                Err(resp) => return resp,
            };
            if let Some(info) = provider.tab_info(idx) {
                let buffer = provider.read_buffer();
                let at_prompt = infer_prompt(&buffer, &info.working_directory);
                // Ensure tab_id_manager is synced for ID lookup
                let mgr = tab_id_manager.lock().unwrap();
                let tab_id = mgr.get_id(idx).unwrap_or("?");
                format!(
                    "STATE|{}|{}|0x{:X}|{}|prompt={}|selection={}|pwd={}|tab_count={}|active_tab={}|tab_id={}\n",
                    session_name,
                    pid,
                    provider.hwnd(),
                    escape_field(&info.title),
                    if at_prompt { '1' } else { '0' },
                    if info.has_selection { '1' } else { '0' },
                    info.working_directory,
                    provider.tab_count(),
                    provider.active_tab(),
                    tab_id,
                )
            } else {
                format!("ERR|{}|invalid-tab\n", session_name)
            }
        }
        Request::Tail { lines, tab } => {
            let tab_index = match resolve_tab(&tab, tab_id_manager, provider, session_name) {
                Ok(idx) => idx,
                Err(resp) => return resp,
            };
            let full_buffer = if let Some(idx) = tab_index {
                match provider.read_buffer_for_tab(idx) {
                    Some(buf) => buf,
                    None => return format!("ERR|{}|invalid-tab\n", session_name),
                }
            } else {
                provider.read_buffer()
            };
            let sliced = slice_last_lines(&full_buffer, lines);
            if sliced.ends_with('\n') {
                format!("TAIL|{}|{}\n{}", session_name, lines, sliced)
            } else {
                format!("TAIL|{}|{}\n{}\n", session_name, lines, sliced)
            }
        }
        Request::ListTabs => {
            let tab_count = provider.tab_count();
            let active_tab = provider.active_tab();

            // Sync the tab ID manager with the actual tab count
            {
                let mut mgr = tab_id_manager.lock().unwrap();
                mgr.sync_tabs(tab_count);
            }

            let mgr = tab_id_manager.lock().unwrap();
            let mut resp = format!("LIST_TABS|{}|{}\n", tab_count, active_tab);
            for i in 0..tab_count {
                if let Some(info) = provider.tab_info(i) {
                    let prompt = if i == active_tab {
                        let buffer = provider.read_buffer();
                        infer_prompt(&buffer, &info.working_directory)
                    } else {
                        false
                    };
                    let tab_id = mgr.get_id(i).unwrap_or("?");
                    resp.push_str(&format!("TAB|{}|{}|{}|pwd={}|prompt={}|selection={}\n",
                        i, tab_id, escape_field(&info.title), info.working_directory,
                        if prompt { '1' } else { '0' },
                        if info.has_selection { '1' } else { '0' }));
                }
            }
            resp
        }
        Request::Input { from: _, payload, tab } => {
            let tab_index = match resolve_tab(&tab, tab_id_manager, provider, session_name) {
                Ok(idx) => idx,
                Err(resp) => return resp,
            };
            match tab_index {
                Some(idx) => provider.send_input_to_tab(&payload, false, idx),
                None => provider.send_input(&payload, false),
            }
            format!("ACK|{}|{}\n", session_name, pid)
        }
        Request::RawInput { from: _, payload, tab } => {
            let tab_index = match resolve_tab(&tab, tab_id_manager, provider, session_name) {
                Ok(idx) => idx,
                Err(resp) => return resp,
            };
            match tab_index {
                Some(idx) => provider.send_input_to_tab(&payload, true, idx),
                None => provider.send_input(&payload, true),
            }
            format!("ACK|{}|{}\n", session_name, pid)
        }
        Request::NewTab => {
            provider.new_tab();
            let new_count = provider.tab_count();
            let mut mgr = tab_id_manager.lock().unwrap();
            mgr.sync_tabs(new_count);
            // The new tab is at index new_count - 1
            let tab_id = mgr.get_id(new_count.saturating_sub(1))
                .unwrap_or("?")
                .to_string();
            format!("OK|{}|NEW_TAB|{}\n", session_name, tab_id)
        }
        Request::CloseTab(tab) => {
            let idx = match resolve_tab_or_active(&tab, tab_id_manager, provider, session_name) {
                Ok(i) => i,
                Err(resp) => return resp,
            };
            // Remove from manager *before* calling close_tab so the index
            // is still valid in the manager's mapping.
            {
                let mut mgr = tab_id_manager.lock().unwrap();
                mgr.remove_tab_at_index(idx);
            }
            provider.close_tab(idx);
            format!("ACK|{}|CLOSE_TAB|{}\n", session_name, idx)
        }
        Request::SwitchTab(tab) => {
            let idx = match resolve_tab_or_active(&tab, tab_id_manager, provider, session_name) {
                Ok(i) => i,
                Err(resp) => return resp,
            };
            provider.switch_tab(idx);
            format!("ACK|{}|SWITCH_TAB|{}\n", session_name, idx)
        }
        Request::Focus => {
            provider.focus();
            format!("ACK|{}|FOCUS\n", session_name)
        }
        Request::AgentStatus => {
            format!("ERR|deprecated|use agent-deck for state detection\n")
        }
        Request::SetAgent { tab: _, agent_type: _ } => {
            format!("ERR|deprecated|use agent-deck for state detection\n")
        }
        Request::Msg(_payload) => {
            format!("ACK|{}|{}\n", session_name, pid)
        }
    }
}
