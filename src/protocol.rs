use base64::{engine::general_purpose::STANDARD, Engine};
use crate::error::Result;

pub enum Request {
    Ping,
    State(Option<usize>),
    Tail(usize),
    ListTabs,
    Input { from: String, payload: Vec<u8> },
    RawInput { from: String, payload: Vec<u8> },
    NewTab,
    CloseTab(Option<usize>),
    SwitchTab(usize),
    Focus,
    AgentStatus,
    SetAgent { tab_index: usize, agent_type: String },
    Msg(String),
}

impl Request {
    pub fn parse(request: &str) -> Result<Self> {
        let parts: Vec<&str> = request.split('|').collect();
        if parts.is_empty() {
            return Err(crate::error::Error::UnknownCommand);
        }

        match parts[0] {
            "PING" => Ok(Request::Ping),
            "STATE" => {
                let tab = if parts.len() > 1 {
                    parts[1].parse().ok()
                } else {
                    None
                };
                Ok(Request::State(tab))
            }
            "TAIL" => {
                let lines = if parts.len() > 1 {
                    parts[1].parse().unwrap_or(20)
                } else {
                    20
                };
                Ok(Request::Tail(lines))
            }
            "LIST_TABS" => Ok(Request::ListTabs),
            "INPUT" => {
                if parts.len() < 3 {
                    return Err(crate::error::Error::InvalidArgument("INPUT|from|payload".to_string()));
                }
                let from = parts[1].to_string();
                let payload = STANDARD.decode(parts[2]).map_err(|_| crate::error::Error::InvalidBase64)?;
                Ok(Request::Input { from, payload })
            }
            "RAW_INPUT" => {
                if parts.len() < 3 {
                    return Err(crate::error::Error::InvalidArgument("RAW_INPUT|from|payload".to_string()));
                }
                let from = parts[1].to_string();
                let payload = STANDARD.decode(parts[2]).map_err(|_| crate::error::Error::InvalidBase64)?;
                Ok(Request::RawInput { from, payload })
            }
            "NEW_TAB" => Ok(Request::NewTab),
            "CLOSE_TAB" => {
                let tab = if parts.len() > 1 {
                    parts[1].parse().ok()
                } else {
                    None
                };
                Ok(Request::CloseTab(tab))
            }
            "SWITCH_TAB" => {
                if parts.len() < 2 {
                    return Err(crate::error::Error::InvalidArgument("SWITCH_TAB|index".to_string()));
                }
                let index = parts[1].parse().map_err(|_| crate::error::Error::InvalidArgument("index".to_string()))?;
                Ok(Request::SwitchTab(index))
            }
            "FOCUS" => Ok(Request::Focus),
            "AGENT_STATUS" => Ok(Request::AgentStatus),
            "SET_AGENT" => {
                if parts.len() < 3 {
                    return Err(crate::error::Error::InvalidArgument("SET_AGENT|tab|type".to_string()));
                }
                let tab_index = parts[1].parse().map_err(|_| crate::error::Error::InvalidArgument("tab".to_string()))?;
                let agent_type = parts[2].to_string();
                Ok(Request::SetAgent { tab_index, agent_type })
            }
            "MSG" => {
                if parts.len() < 2 {
                    return Err(crate::error::Error::InvalidArgument("MSG|payload".to_string()));
                }
                Ok(Request::Msg(parts[1].to_string()))
            }
            _ => Err(crate::error::Error::UnknownCommand),
        }
    }
}

pub fn escape_field(value: &str) -> String {
    value.chars().map(|c| if c == '|' || c == '\r' || c == '\n' { ' ' } else { c }).collect()
}
