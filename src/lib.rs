pub mod error;
pub mod protocol;
pub mod server;
pub mod agent_status;
pub mod session;
pub mod tab_id;
pub mod utils;
pub mod ffi;

pub use error::{Error, Result};
pub use server::ControlPlaneServer;

#[derive(Debug, Clone, Default)]
pub struct TabInfo {
    pub title: String,
    pub working_directory: String,
    pub has_selection: bool,
}

pub trait TerminalProvider: Send + Sync {
    /// Get the entire terminal buffer or at least the last few hundred lines.
    fn read_buffer(&self) -> String;
    /// Inject text input.
    fn send_input(&self, text: &[u8], raw: bool);
    /// Get total number of tabs.
    fn tab_count(&self) -> usize;
    /// Get the index of the active tab.
    fn active_tab(&self) -> usize;
    /// Get info for a specific tab.
    fn tab_info(&self, index: usize) -> Option<TabInfo>;
    /// Switch to a specific tab.
    fn switch_tab(&self, index: usize);
    /// Open a new tab.
    fn new_tab(&self);
    /// Close a specific tab.
    fn close_tab(&self, index: usize);
    /// Focus the terminal window.
    fn focus(&self);
    /// Get the HWND of the terminal window (0 if unavailable).
    fn hwnd(&self) -> usize;
    /// Read buffer for a specific tab (None = not supported or invalid index).
    fn read_buffer_for_tab(&self, _index: usize) -> Option<String> { None }
    /// Send input to a specific tab. Default: switch tab, send, switch back.
    fn send_input_to_tab(&self, text: &[u8], raw: bool, tab_index: usize) {
        let original = self.active_tab();
        if tab_index != original {
            self.switch_tab(tab_index);
        }
        self.send_input(text, raw);
        if tab_index != original {
            self.switch_tab(original);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use crate::server::ControlPlaneServer;

    struct MockProvider {
        buffer: String,
    }

    impl TerminalProvider for MockProvider {
        fn read_buffer(&self) -> String { self.buffer.clone() }
        fn send_input(&self, _text: &[u8], _raw: bool) {}
        fn tab_count(&self) -> usize { 1 }
        fn active_tab(&self) -> usize { 0 }
        fn tab_info(&self, _index: usize) -> Option<TabInfo> {
            Some(TabInfo {
                title: "Mock".to_string(),
                working_directory: "/mock".to_string(),
                has_selection: false,
            })
        }
        fn switch_tab(&self, _index: usize) {}
        fn new_tab(&self) {}
        fn close_tab(&self, _index: usize) {}
        fn focus(&self) {}
        fn hwnd(&self) -> usize { 0 }
    }

    #[test]
    #[ignore] // Requires running pipe server
    fn test_server_real_pipe() {
        use std::io::{Read, Write};
        
        let provider = Arc::new(MockProvider { buffer: "hello".to_string() });
        let server = ControlPlaneServer::new(provider, "test-pipe-session".to_string(), None).unwrap();
        
        // Start server
        server.start().unwrap();
        
        // Give the server a moment to create the pipe
        std::thread::sleep(Duration::from_millis(200));
        
        // Connect as a client using standard Rust File (can open pipes)
        // Note: We use the pipe path from server
        let pipe_path = format!(r"\\.\pipe\windows-terminal-winui3-test-pipe-session-{}-rs", unsafe { windows::Win32::System::Threading::GetCurrentProcessId() });
        
        let mut client = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe_path)
            .expect("Failed to connect to pipe");

        // Send PING
        client.write_all(b"PING").unwrap();
        client.flush().unwrap();

        // Read response
        let mut resp = [0u8; 128];
        let n = client.read(&mut resp).unwrap();
        let resp_str = String::from_utf8_lossy(&resp[..n]);
        
        assert!(resp_str.starts_with("PONG|test-pipe-session|"));
        
        server.stop();
    }

    #[test]
    fn test_agent_status_ready() {
        use crate::agent_status::StatusEngine;
        let mut engine = StatusEngine::new();
        let provider = MockProvider { buffer: "Type your message".to_string() };
        engine.set_agent_type(0, "gemini".to_string());
        
        let (status, _ms, tab) = engine.get_status(&provider);
        assert_eq!(status.as_str(), "READY");
        assert_eq!(tab, 0);
    }

    #[test]
    fn test_prompt_inference() {
        use crate::utils::infer_prompt;
        assert!(infer_prompt("C:\\Users\\yuuji> ", "C:\\Users\\yuuji"));
        assert!(infer_prompt("bash$ ", "/home/user"));
        assert!(!infer_prompt("hello world", "/"));
    }
}
