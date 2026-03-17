use std::fs;
use std::io::Write;
use std::path::PathBuf;
use crate::error::{Error, Result};

pub struct SessionManager {
    pub session_name: String,
    pub safe_session_name: String,
    pub pid: u32,
    pub pipe_path: String,
    pub session_file: PathBuf,
}

impl SessionManager {
    pub fn new(session_name: String, pipe_path: String) -> Result<Self> {
        let pid = unsafe { windows::Win32::System::Threading::GetCurrentProcessId() };
        let safe_session_name = sanitize_session_name(&session_name);
        
        let local_app_data = std::env::var("LOCALAPPDATA")
            .map_err(|_| Error::InvalidArgument("LOCALAPPDATA not set".to_string()))?;
        
        let mut session_dir = PathBuf::from(local_app_data);
        session_dir.push("WindowsTerminal");
        session_dir.push("control-plane");
        session_dir.push("winui3");
        session_dir.push("sessions");

        fs::create_dir_all(&session_dir)?;

        let session_file = session_dir.join(format!("{}-{}.session", safe_session_name, pid));

        Ok(Self {
            session_name,
            safe_session_name,
            pid,
            pipe_path,
            session_file,
        })
    }

    pub fn write_file(&self, hwnd: usize) -> Result<()> {
        let mut f = fs::File::create(&self.session_file)?;
        writeln!(f, "session_name={}", self.session_name)?;
        writeln!(f, "safe_session_name={}", self.safe_session_name)?;
        writeln!(f, "pid={}", self.pid)?;
        writeln!(f, "hwnd=0x{:X}", hwnd)?;
        writeln!(f, "pipe_path={}", self.pipe_path)?;
        Ok(())
    }

    pub fn remove_file(&self) {
        let _ = fs::remove_file(&self.session_file);
    }
}

pub fn sanitize_session_name(name: &str) -> String {
    let mut sanitized: String = name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    sanitized = sanitized.trim_matches('_').to_string();
    if sanitized.is_empty() {
        "session".to_string()
    } else {
        sanitized
    }
}
