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
use crate::protocol::{Request, escape_field};
use crate::agent_status::StatusEngine;
use crate::session::{SessionManager, sanitize_session_name};
use crate::TerminalProvider;
use crate::utils::{slice_last_lines, infer_prompt};

const MAX_READ_SIZE: u32 = 65536;

pub struct ControlPlaneServer {
    provider: Arc<dyn TerminalProvider>,
    status_engine: Arc<Mutex<StatusEngine>>,
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

        let session_manager = Arc::new(SessionManager::new(session_name, pipe_path)?);

        Ok(Self {
            provider,
            status_engine: Arc::new(Mutex::new(StatusEngine::new())),
            session_manager,
            stop: Arc::new(Mutex::new(false)),
        })
    }

    pub fn start(&self) -> Result<()> {
        let hwnd = self.provider.hwnd();
        self.session_manager.write_file(hwnd)?;

        let provider = self.provider.clone();
        let status_engine = self.status_engine.clone();
        let pipe_path = self.session_manager.pipe_path.clone();
        let session_name = self.session_manager.session_name.clone();
        let pid = self.session_manager.pid;
        let stop = self.stop.clone();

        thread::spawn(move || {
            if let Err(e) = server_thread_main(provider, status_engine, pipe_path, session_name, pid, stop) {
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
    status_engine: Arc<Mutex<StatusEngine>>,
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
            handle_client(pipe, &provider, &status_engine, &session_name, pid);
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
    status_engine: &Arc<Mutex<StatusEngine>>,
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
            let response = build_response(trimmed, provider, status_engine, session_name, pid);
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

fn build_response(
    request_str: &str,
    provider: &Arc<dyn TerminalProvider>,
    status_engine: &Arc<Mutex<StatusEngine>>,
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
        Request::State(tab_idx) => {
            let idx = tab_idx.unwrap_or_else(|| provider.active_tab());
            if let Some(info) = provider.tab_info(idx) {
                let buffer = provider.read_buffer();
                let at_prompt = infer_prompt(&buffer, &info.working_directory);
                format!(
                    "STATE|{}|{}|0x{:X}|{}|prompt={}|selection={}|pwd={}|tab_count={}|active_tab={}\n",
                    session_name,
                    pid,
                    provider.hwnd(),
                    escape_field(&info.title),
                    if at_prompt { '1' } else { '0' },
                    if info.has_selection { '1' } else { '0' },
                    info.working_directory,
                    provider.tab_count(),
                    provider.active_tab()
                )
            } else {
                format!("ERR|{}|invalid-tab\n", session_name)
            }
        }
        Request::Tail(lines) => {
            let full_buffer = provider.read_buffer();
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
            let mut resp = format!("LIST_TABS|{}|{}\n", tab_count, active_tab);
            for i in 0..tab_count {
                if let Some(info) = provider.tab_info(i) {
                    let prompt = if i == active_tab {
                        let buffer = provider.read_buffer();
                        infer_prompt(&buffer, &info.working_directory)
                    } else {
                        false
                    };
                    resp.push_str(&format!("TAB|{}|{}|pwd={}|prompt={}|selection={}\n",
                        i, escape_field(&info.title), info.working_directory,
                        if prompt { '1' } else { '0' },
                        if info.has_selection { '1' } else { '0' }));
                }
            }
            resp
        }
        Request::Input { from: _, payload } => {
            provider.send_input(&payload, false);
            format!("ACK|{}|{}\n", session_name, pid)
        }
        Request::RawInput { from: _, payload } => {
            provider.send_input(&payload, true);
            format!("ACK|{}|{}\n", session_name, pid)
        }
        Request::NewTab => {
            provider.new_tab();
            format!("ACK|{}|NEW_TAB\n", session_name)
        }
        Request::CloseTab(idx) => {
            let idx = idx.unwrap_or_else(|| provider.active_tab());
            provider.close_tab(idx);
            format!("ACK|{}|CLOSE_TAB|{}\n", session_name, idx)
        }
        Request::SwitchTab(idx) => {
            provider.switch_tab(idx);
            format!("ACK|{}|SWITCH_TAB|{}\n", session_name, idx)
        }
        Request::Focus => {
            provider.focus();
            format!("ACK|{}|FOCUS\n", session_name)
        }
        Request::AgentStatus => {
            let mut engine = status_engine.lock().unwrap();
            let (status, ms, tab) = engine.get_status(&**provider);
            format!("AGENT_STATUS|{}|{}|{}|tab={}\n", session_name, status.as_str(), ms, tab)
        }
        Request::SetAgent { tab_index, agent_type } => {
            let mut engine = status_engine.lock().unwrap();
            engine.set_agent_type(tab_index, agent_type.clone());
            format!("ACK|{}|SET_AGENT|{}|{}\n", session_name, tab_index, agent_type)
        }
        Request::Msg(_payload) => {
            format!("ACK|{}|{}\n", session_name, pid)
        }
    }
}
