use std::collections::HashMap;
use std::time::Instant;
use crate::TerminalProvider;
use crate::utils::slice_last_lines;

#[derive(Default)]
pub struct StatusEngine {
    tab_snapshots: HashMap<usize, String>,
    tab_change_times: HashMap<usize, Instant>,
    tab_agent_types: HashMap<usize, String>,
}

pub enum AgentStatus {
    Idle,
    Working,
    Starting,
    Ready,
    Approval,
}

impl AgentStatus {
    pub fn as_str(&self) -> &str {
        match self {
            AgentStatus::Idle => "IDLE",
            AgentStatus::Working => "WORKING",
            AgentStatus::Starting => "STARTING",
            AgentStatus::Ready => "READY",
            AgentStatus::Approval => "APPROVAL",
        }
    }
}

impl StatusEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_agent_type(&mut self, tab_index: usize, agent_type: String) {
        self.tab_agent_types.insert(tab_index, agent_type);
    }

    pub fn get_status(&mut self, provider: &dyn TerminalProvider) -> (AgentStatus, u64, usize) {
        let tab_index = provider.active_tab();
        let full_buffer = provider.read_buffer();
        let current_buffer = slice_last_lines(&full_buffer, 10).to_string();
        let now = Instant::now();

        let last_snapshot = self.tab_snapshots.entry(tab_index).or_insert_with(|| current_buffer.clone());
        let last_change_time = self.tab_change_times.entry(tab_index).or_insert(now);

        let buffer_changed = current_buffer != *last_snapshot;
        if buffer_changed {
            *last_snapshot = current_buffer.clone();
            *last_change_time = now;
        }

        let agent_type = self.tab_agent_types.get(&tab_index).cloned();
        let agent_ready = if let Some(ref atype) = agent_type {
            match atype.as_str() {
                "gemini" => current_buffer.contains("Type your message"),
                "codex" => current_buffer.contains("left \u{00b7} ~") || current_buffer.contains("\u{203a} "),
                "claude" => current_buffer.contains("$ ") || current_buffer.contains("> "),
                _ => false,
            }
        } else {
            false
        };

        let status = if current_buffer.contains("Allow once") || current_buffer.contains("Action Required") || current_buffer.contains("Would you like to run") {
            AgentStatus::Approval
        } else if buffer_changed {
            AgentStatus::Working
        } else if agent_type.is_some() && !agent_ready {
            AgentStatus::Starting
        } else if agent_type.is_some() && agent_ready {
            AgentStatus::Ready
        } else {
            AgentStatus::Idle
        };

        let ms = last_change_time.elapsed().as_millis() as u64;
        (status, ms, tab_index)
    }
}
