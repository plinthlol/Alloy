// in-memory ring buffer for live logs of running instances, capped at
// 2000 lines each so a week-long server session doesn't eat all the RAM.

use std::collections::{HashMap, VecDeque};
use std::sync::LazyLock;
use std::sync::{Arc, Mutex};

use crate::instance::launch::parser::{LogLevel, LogStream, ParsedLogEvent};

const MAX_LINES: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveLogLine {
    pub level: LogLevel,
    pub stream: LogStream,
    pub text: String,
}

type LogsMap = Arc<Mutex<HashMap<String, VecDeque<LiveLogLine>>>>;
pub static LOGS: LazyLock<LogsMap> = LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

pub fn push_event(name: &str, event: ParsedLogEvent) {
    for line in event.lines {
        push_line(name, event.level, event.primary_stream, line);
    }
}

pub fn push_line(name: &str, level: LogLevel, stream: LogStream, line: impl Into<String>) {
    if let Ok(mut logs) = LOGS.lock() {
        let buf = logs.entry(name.to_string()).or_insert_with(VecDeque::new);
        buf.push_back(LiveLogLine {
            level,
            stream,
            text: line.into(),
        });
        while buf.len() > MAX_LINES {
            buf.pop_front();
        }
    }
}

pub fn get_entries(name: &str) -> Vec<LiveLogLine> {
    LOGS.lock()
        .ok()
        .and_then(|logs| logs.get(name).map(|buf| buf.iter().cloned().collect()))
        .unwrap_or_default()
}

pub fn clear(name: &str) {
    if let Ok(mut logs) = LOGS.lock() {
        logs.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_line_and_get_entries() {
        let name = "test_push_get";
        push_line(name, LogLevel::Info, LogStream::Stdout, "line1");
        push_line(name, LogLevel::Info, LogStream::Stdout, "line2");
        let entries = get_entries(name);
        let texts: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();
        assert!(texts.contains(&"line1"));
        assert!(texts.contains(&"line2"));
    }

    #[test]
    fn get_entries_missing_instance_returns_empty() {
        let entries = get_entries("nonexistent_instance_xyz");
        assert!(entries.is_empty());
    }

    #[test]
    fn clear_removes_instance() {
        let name = "test_clear";
        push_line(name, LogLevel::Info, LogStream::Stdout, "data");
        assert!(!get_entries(name).is_empty());
        clear(name);
        assert!(get_entries(name).is_empty());
    }

    #[test]
    fn clear_nonexistent_is_noop() {
        clear("never_existed_xyz");
    }

    #[test]
    fn buffer_respects_max_lines() {
        let name = "test_max_lines";
        for i in 0..(MAX_LINES + 100) {
            push_line(name, LogLevel::Info, LogStream::Stdout, format!("line-{i}"));
        }
        let entries = get_entries(name);
        assert_eq!(entries.len(), MAX_LINES);
        assert!(
            entries
                .last()
                .unwrap()
                .text
                .contains(&format!("{}", MAX_LINES + 99))
        );
    }
}
