#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_by_level_component_and_id() {
        let line = discard_core::log_view::parse_line(r#"{"level":"WARN","target":"discard_mcp","fields":{"session":"abc"}}"#);
        let selected = filter("WARN", "MCP", "bc").unwrap();
        assert!(selected.matches(&line));
        assert!(!filter("ERROR", "MCP", "bc").unwrap().matches(&line));
    }
}

use discard_core::log_view::{self, Component, Level, LogFilter, LogLine, LogTail, MAX_LINES};
/// Incremental log tail and its bounded cache.
pub struct LogState { tail: LogTail, lines: Vec<LogLine> }
impl LogState {
    pub fn open(home: &std::path::Path) -> Self { Self { tail: LogTail::open(&home.join("logs")), lines: Vec::new() } }
    fn poll(&mut self) {
        let generation = self.tail.generation();
        let lines = self.tail.poll();
        if generation != self.tail.generation() { self.lines.clear(); }
        self.lines.extend(lines.iter().map(|s| log_view::parse_line(s)));
        if self.lines.len() > MAX_LINES { self.lines.drain(..self.lines.len() - MAX_LINES); }
    }
}
fn filter(level: &str, component: &str, id: &str) -> Result<LogFilter, String> {
    let min_level = Level::parse(level).ok_or("Unknown log level")?;
    let component = match component { "All" => None, "Provider" => Some(Component::Provider), "Plugin" => Some(Component::Plugin), "Approval" => Some(Component::Approval), "Storage" => Some(Component::Storage), "MCP" => Some(Component::Mcp), "Workspace" => Some(Component::Workspace), "Other" => Some(Component::Other), _ => return Err("Unknown log component".into()) };
    Ok(LogFilter { min_level, component, id: id.into() })
}
#[tauri::command]
pub fn settings_logs(context: tauri::State<'_, super::SettingsContext>, level: String, component: String, id: String) -> Result<Vec<String>, String> {
    let filter = filter(&level, &component, &id)?;
    let mut logs = context.logs.lock().unwrap_or_else(|e| e.into_inner());
    logs.poll();
    Ok(logs.lines.iter().filter(|line| filter.matches(line)).map(|line| log_view::shorten(&format!("{} {:?} {} {}", line.time, line.level, line.component.label(), line.message))).collect())
}
