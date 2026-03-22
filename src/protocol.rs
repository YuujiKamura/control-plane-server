use base64::{engine::general_purpose::STANDARD, Engine};
use crate::error::Result;

/// How a command addresses a tab: by stable ID, by positional index, or
/// not at all (meaning "use the active tab").
#[derive(Debug, Clone, PartialEq)]
pub enum TabTarget {
    /// Stable ID, e.g. `id=t_001`
    Id(String),
    /// Legacy positional index
    Index(usize),
    /// No tab specified – use default / active tab
    None,
}

impl TabTarget {
    /// Parse a single pipe-delimited field that may be `id=<tab_id>` or a
    /// plain integer index.
    pub fn parse(s: &str) -> Self {
        if let Some(id) = s.strip_prefix("id=") {
            TabTarget::Id(id.to_string())
        } else if let Ok(idx) = s.parse::<usize>() {
            TabTarget::Index(idx)
        } else {
            TabTarget::None
        }
    }

    /// Parse an optional field (returns `TabTarget::None` when absent).
    pub fn parse_optional(parts: &[&str], pos: usize) -> Self {
        if parts.len() > pos {
            Self::parse(parts[pos])
        } else {
            TabTarget::None
        }
    }
}

pub enum Request {
    Ping,
    State(TabTarget),
    Tail { lines: usize, tab: TabTarget },
    ListTabs,
    Input { from: String, payload: Vec<u8>, tab: TabTarget },
    RawInput { from: String, payload: Vec<u8>, tab: TabTarget },
    NewTab,
    CloseTab(TabTarget),
    SwitchTab(TabTarget),
    Focus,
    AgentStatus,
    SetAgent { tab: TabTarget, agent_type: String },
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
                let tab = TabTarget::parse_optional(&parts, 1);
                Ok(Request::State(tab))
            }
            "TAIL" => {
                let lines = if parts.len() > 1 {
                    parts[1].parse().unwrap_or(20)
                } else {
                    20
                };
                let tab = TabTarget::parse_optional(&parts, 2);
                Ok(Request::Tail { lines, tab })
            }
            "LIST_TABS" => Ok(Request::ListTabs),
            "INPUT" => {
                if parts.len() < 3 {
                    return Err(crate::error::Error::InvalidArgument("INPUT|from|payload[|tab]".to_string()));
                }
                let from = parts[1].to_string();
                let payload = STANDARD.decode(parts[2]).map_err(|_| crate::error::Error::InvalidBase64)?;
                let tab = TabTarget::parse_optional(&parts, 3);
                Ok(Request::Input { from, payload, tab })
            }
            "RAW_INPUT" => {
                if parts.len() < 3 {
                    return Err(crate::error::Error::InvalidArgument("RAW_INPUT|from|payload[|tab]".to_string()));
                }
                let from = parts[1].to_string();
                let payload = STANDARD.decode(parts[2]).map_err(|_| crate::error::Error::InvalidBase64)?;
                let tab = TabTarget::parse_optional(&parts, 3);
                Ok(Request::RawInput { from, payload, tab })
            }
            "NEW_TAB" => Ok(Request::NewTab),
            "CLOSE_TAB" => {
                let tab = TabTarget::parse_optional(&parts, 1);
                Ok(Request::CloseTab(tab))
            }
            "SWITCH_TAB" => {
                if parts.len() < 2 {
                    return Err(crate::error::Error::InvalidArgument("SWITCH_TAB|index_or_id".to_string()));
                }
                let tab = TabTarget::parse(parts[1]);
                Ok(Request::SwitchTab(tab))
            }
            "FOCUS" => Ok(Request::Focus),
            "AGENT_STATUS" => Ok(Request::AgentStatus),
            "SET_AGENT" => {
                if parts.len() < 3 {
                    return Err(crate::error::Error::InvalidArgument("SET_AGENT|tab|type".to_string()));
                }
                let tab = TabTarget::parse(parts[1]);
                let agent_type = parts[2].to_string();
                Ok(Request::SetAgent { tab, agent_type })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_target_parse_id() {
        assert_eq!(TabTarget::parse("id=t_001"), TabTarget::Id("t_001".to_string()));
    }

    #[test]
    fn test_tab_target_parse_index() {
        assert_eq!(TabTarget::parse("2"), TabTarget::Index(2));
    }

    #[test]
    fn test_tab_target_parse_garbage() {
        assert_eq!(TabTarget::parse("xyz"), TabTarget::None);
    }

    #[test]
    fn test_parse_tail_with_id() {
        let req = Request::parse("TAIL|50|id=t_002").unwrap();
        match req {
            Request::Tail { lines, tab } => {
                assert_eq!(lines, 50);
                assert_eq!(tab, TabTarget::Id("t_002".to_string()));
            }
            _ => panic!("expected Tail"),
        }
    }

    #[test]
    fn test_parse_tail_with_index() {
        let req = Request::parse("TAIL|20|1").unwrap();
        match req {
            Request::Tail { lines, tab } => {
                assert_eq!(lines, 20);
                assert_eq!(tab, TabTarget::Index(1));
            }
            _ => panic!("expected Tail"),
        }
    }

    #[test]
    fn test_parse_close_tab_with_id() {
        let req = Request::parse("CLOSE_TAB|id=t_000").unwrap();
        match req {
            Request::CloseTab(tab) => assert_eq!(tab, TabTarget::Id("t_000".to_string())),
            _ => panic!("expected CloseTab"),
        }
    }

    #[test]
    fn test_parse_switch_tab_with_id() {
        let req = Request::parse("SWITCH_TAB|id=t_003").unwrap();
        match req {
            Request::SwitchTab(tab) => assert_eq!(tab, TabTarget::Id("t_003".to_string())),
            _ => panic!("expected SwitchTab"),
        }
    }

    #[test]
    fn test_parse_set_agent_with_id() {
        let req = Request::parse("SET_AGENT|id=t_001|claude").unwrap();
        match req {
            Request::SetAgent { tab, agent_type } => {
                assert_eq!(tab, TabTarget::Id("t_001".to_string()));
                assert_eq!(agent_type, "claude");
            }
            _ => panic!("expected SetAgent"),
        }
    }

    #[test]
    fn test_backward_compat_input() {
        let req = Request::parse("INPUT|user|aGVsbG8=|2").unwrap();
        match req {
            Request::Input { from, tab, .. } => {
                assert_eq!(from, "user");
                assert_eq!(tab, TabTarget::Index(2));
            }
            _ => panic!("expected Input"),
        }
    }
}
