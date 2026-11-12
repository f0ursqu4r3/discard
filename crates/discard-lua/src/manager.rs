//! Owns every loaded plugin. Resolves slots (the last plugin to register a slot wins, and an earlier one
//! or the built-in Rust view takes over if it fails), merges themes, and tracks errors for the error badge.

use crate::element::Element;
use crate::host::Host;
use crate::runtime::{CommandInfo, HookResult, LoadedInfo, MenuItemInfo, PluginError, PluginHandle, PluginLimits, Status};
use crate::snapshot::Snapshot;
use crate::theme::ThemeOverrides;
use std::collections::BTreeMap;

/// The result of rendering a slot: which plugin produced it, so UI actions can be routed back.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub plugin: String,
    pub element: Element,
}

struct Entry {
    handle: PluginHandle,
    info: LoadedInfo,
    last_error: Option<String>,
    /// Failures since the last success. A terminated plugin counts as `TERMINATED_FAILURES`.
    failures: u32,
}

/// A terminated plugin is past saving, so it counts as enough failures to trigger an auto-revert.
pub const TERMINATED_FAILURES: u32 = 3;

impl Entry {
    fn succeeded(&mut self) {
        self.last_error = None;
        self.failures = 0;
    }

    fn failed(&mut self, error: &PluginError) {
        self.last_error = Some(error.to_string());
        self.failures = if matches!(error, PluginError::Violation(_) | PluginError::Terminated(_)) {
            TERMINATED_FAILURES
        } else {
            self.failures + 1
        };
    }
}

pub struct PluginManager {
    limits: PluginLimits,
    host: Host,
    plugins: Vec<Entry>,
}

impl PluginManager {
    /// A manager whose plugins get no files and no memory. Use it to check drafts.
    pub fn new(limits: PluginLimits) -> Self {
        Self::with_host(limits, Host::default())
    }

    /// A manager whose plugins get the services their manifests ask for.
    pub fn with_host(limits: PluginLimits, host: Host) -> Self {
        Self { limits, host, plugins: Vec::new() }
    }

    /// Load a plugin. A plugin with the same name is replaced. Later plugins win over earlier ones.
    pub fn load(&mut self, snapshot: Snapshot) -> Result<LoadedInfo, PluginError> {
        // Spawn first. If it fails, nothing changes and an older version of the plugin keeps running.
        let (handle, info) = PluginHandle::spawn_with_host(snapshot, self.limits.clone(), self.host.clone())?;
        let entry = Entry { handle, info: info.clone(), last_error: None, failures: 0 };
        match self.plugins.iter().position(|e| e.info.manifest.name == info.manifest.name) {
            Some(i) => {
                let old = std::mem::replace(&mut self.plugins[i], entry);
                old.handle.shutdown();
            }
            None => self.plugins.push(entry),
        }
        Ok(info)
    }

    pub fn unload(&mut self, name: &str) -> bool {
        match self.plugins.iter().position(|e| e.info.manifest.name == name) {
            Some(i) => {
                self.plugins.remove(i).handle.shutdown();
                true
            }
            None => false,
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.plugins.iter().map(|e| e.info.manifest.name.clone()).collect()
    }

    /// Every slot name any loaded plugin registered, sorted.
    pub fn slot_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.plugins.iter().flat_map(|e| e.info.slots.iter().cloned()).collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn info(&self, name: &str) -> Option<&LoadedInfo> {
        self.plugins.iter().find(|e| e.info.manifest.name == name).map(|e| &e.info)
    }

    /// Render a slot. Tries the last registered plugin first. `None` means the built-in view should draw it.
    pub fn render_slot(&mut self, slot: &str, state: &serde_json::Value) -> Option<Rendered> {
        for entry in self.plugins.iter_mut().rev() {
            if !entry.info.slots.iter().any(|n| n == slot) || entry.handle.status() != Status::Running {
                continue;
            }
            match entry.handle.render(slot, state) {
                Ok(element) => {
                    entry.succeeded();
                    return Some(Rendered { plugin: entry.info.manifest.name.clone(), element });
                }
                Err(e) => entry.failed(&e),
            }
        }
        None
    }

    /// Merge the themes of all running plugins, in load order.
    pub fn theme(&self) -> ThemeOverrides {
        let mut merged = ThemeOverrides::default();
        for entry in self.plugins.iter().filter(|e| e.handle.status() == Status::Running) {
            merged.merge(&entry.info.theme);
        }
        merged
    }

    /// Each theme token a running plugin sets, with the plugin that wins it. Sorted by token.
    pub fn theme_sources(&self) -> Vec<(String, String)> {
        let mut sources = std::collections::BTreeMap::new();
        for entry in self.plugins.iter().filter(|e| e.handle.status() == Status::Running) {
            for token in entry.info.theme.token_names() {
                sources.insert(token, entry.info.manifest.name.clone());
            }
        }
        sources.into_iter().collect()
    }

    pub fn run_action(&mut self, plugin: &str, id: &str) -> Result<(), PluginError> {
        let entry = self
            .plugins
            .iter_mut()
            .find(|e| e.info.manifest.name == plugin)
            .ok_or_else(|| PluginError::NotRegistered { kind: "plugin", name: plugin.to_string() })?;
        let result = entry.handle.run_action(id);
        match &result {
            Ok(()) => entry.succeeded(),
            Err(e) => entry.failed(e),
        }
        result
    }

    pub fn menu_items(&self) -> Vec<(String, usize, MenuItemInfo)> {
        self.plugins.iter().filter(|entry| entry.handle.status() == Status::Running).flat_map(|entry| entry.info.menu_items.iter().cloned().enumerate().map(|(index, info)| (entry.info.manifest.name.clone(), index, info))).collect()
    }

    pub fn run_menu_item(&mut self, plugin: &str, index: usize, kind: &str, id: &str) -> Result<(), PluginError> {
        let entry = self.plugins.iter_mut().find(|entry| entry.info.manifest.name == plugin).ok_or_else(|| PluginError::NotRegistered { kind: "plugin", name: plugin.into() })?;
        let result = entry.handle.run_menu_item(index, kind, id);
        match &result { Ok(()) => entry.succeeded(), Err(error) => entry.failed(error) }
        result
    }

    /// Commands of every plugin with the status `Running`, in load order. `(plugin name, command)`.
    pub fn commands(&self) -> Vec<(String, CommandInfo)> {
        self.plugins.iter().filter(|entry| entry.handle.status() == Status::Running).flat_map(|entry| entry.info.commands.iter().map(|command| (entry.info.manifest.name.clone(), command.clone()))).collect()
    }

    /// Run a slash command. The first plugin that registered it handles it.
    pub fn run_command(&mut self, name: &str, args: &str) -> Option<Result<Option<String>, PluginError>> {
        let entry = self.plugins.iter_mut().find(|e| e.handle.status() == Status::Running && e.info.commands.iter().any(|c| c.name == name))?;
        let result = entry.handle.run_command(name, args);
        match &result {
            Ok(_) => entry.succeeded(),
            Err(e) => entry.failed(e),
        }
        Some(result)
    }

    pub fn hook(&mut self, event: &str, payload: &serde_json::Value) -> Vec<(String, Result<HookResult, PluginError>)> {
        self.plugins
            .iter_mut()
            .filter(|e| e.info.events.iter().any(|n| n == event))
            .map(|e| {
                let result = e.handle.hook(event, payload);
                match &result {
                    Ok(_) => e.succeeded(),
                    Err(err) => e.failed(err),
                }
                (e.info.manifest.name.clone(), result)
            })
            .collect()
    }

    pub fn emit(&self, event: &str, payload: &serde_json::Value) {
        for e in &self.plugins {
            if e.info.events.iter().any(|n| n == event) {
                e.handle.emit(event, payload);
            }
        }
    }

    /// Plugins that failed, for the Rust-rendered plugin error list.
    pub fn errors(&self) -> BTreeMap<String, String> {
        self.plugins
            .iter()
            .filter_map(|e| {
                let message = match e.handle.status() {
                    Status::Terminated(reason) => Some(reason),
                    Status::Running => e.last_error.clone(),
                };
                message.map(|m| (e.info.manifest.name.clone(), m))
            })
            .collect()
    }

    /// Failures in a row for a plugin. Zero after a success.
    pub fn failure_count(&self, name: &str) -> u32 {
        self.plugins.iter().find(|e| e.info.manifest.name == name).map_or(0, |e| e.failures)
    }

    /// Plugins that have failed enough times in a row that Discard should revert them.
    pub fn failing(&self, threshold: u32) -> Vec<String> {
        self.plugins.iter().filter(|e| e.failures >= threshold).map(|e| e.info.manifest.name.clone()).collect()
    }

    pub fn status(&self, name: &str) -> Option<Status> {
        self.plugins.iter().find(|e| e.info.manifest.name == name).map(|e| e.handle.status())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{Node, Tag};
    use crate::theme::Rgb;
    use serde_json::json;
    use std::time::Duration;

    fn plugin(name: &str, caps: &str, init: &str) -> Snapshot {
        let manifest = format!("name = \"{name}\"\nversion = \"1\"\napi = \"1\"\n{caps}");
        Snapshot::from_files(
            [("plugin.toml".to_string(), manifest.into_bytes()), ("init.lua".to_string(), init.as_bytes().to_vec())].into(),
        )
    }

    const UI: &str = "[capabilities]\nui = true\n";

    fn manager() -> PluginManager {
        PluginManager::new(PluginLimits { call_budget: Duration::from_millis(150), ..PluginLimits::default() })
    }

    #[test]
    fn menu_items_are_listed_in_load_order_and_errors_are_counted() {
        let mut m = manager();
        m.load(plugin("a", UI, "discard.menu_item('app', 'One', function() error('oops') end) discard.menu_item('channel', 'Two', function() end)")).unwrap();
        m.load(plugin("b", UI, "discard.menu_item('app', 'Three', function() end)")).unwrap();
        let items = m.menu_items();
        assert_eq!(items.iter().map(|(name, index, info)| (name.as_str(), *index, info.label.as_str())).collect::<Vec<_>>(), [("a", 0, "One"), ("a", 1, "Two"), ("b", 0, "Three")]);
        assert!(m.run_menu_item("a", 0, "app", "").is_err());
        assert_eq!(m.failure_count("a"), 1);
    }

    #[test]
    fn menu_items_leave_the_list_when_the_plugin_is_terminated() {
        let mut m = manager();
        m.load(plugin("loop", UI, "discard.menu_item('app', 'Run', function() while true do end end)")).unwrap();
        assert_eq!(m.menu_items().len(), 1);
        assert!(m.run_menu_item("loop", 0, "app", "").is_err());
        assert!(m.menu_items().is_empty());
    }

    fn text_slot(slot: &str, text: &str) -> String {
        format!("discard.slot(\"{slot}\", function() return {{ \"text\", \"{text}\" }} end)")
    }

    fn first_text(r: &Rendered) -> String {
        match &r.element.children[0] {
            Node::Text(t) => t.clone(),
            _ => String::new(),
        }
    }

    #[test]
    fn the_last_plugin_to_register_a_slot_wins() {
        let mut m = manager();
        m.load(plugin("one", UI, &text_slot("header", "from one"))).unwrap();
        m.load(plugin("two", UI, &text_slot("header", "from two"))).unwrap();
        let r = m.render_slot("header", &json!({})).unwrap();
        assert_eq!((r.plugin.as_str(), first_text(&r).as_str()), ("two", "from two"));
        assert_eq!(r.element.tag, Tag::Text);
    }

    #[test]
    fn an_unregistered_slot_falls_through_to_the_builtin_view() {
        let mut m = manager();
        m.load(plugin("one", UI, &text_slot("header", "x"))).unwrap();
        assert_eq!(m.render_slot("footer", &json!({})), None);
        assert_eq!(manager().render_slot("header", &json!({})), None);
    }

    #[test]
    fn a_failing_plugin_falls_back_to_the_previous_one_and_is_listed_as_failed() {
        let mut m = manager();
        m.load(plugin("good", UI, &text_slot("header", "good"))).unwrap();
        m.load(plugin("bad", UI, "discard.slot(\"header\", function() error(\"kaboom\") end)")).unwrap();
        let r = m.render_slot("header", &json!({})).unwrap();
        assert_eq!(r.plugin, "good");
        assert!(m.errors()["bad"].contains("kaboom"));
        assert!(!m.errors().contains_key("good"));
    }

    #[test]
    fn when_every_provider_fails_the_builtin_view_takes_over() {
        let mut m = manager();
        m.load(plugin("bad", UI, "discard.slot(\"header\", function() error(\"x\") end)")).unwrap();
        assert_eq!(m.render_slot("header", &json!({})), None);
    }

    #[test]
    fn a_successful_render_clears_that_plugins_error() {
        let mut m = manager();
        m.load(plugin("flaky", UI, "local n = 0 discard.slot(\"h\", function() n = n + 1 if n == 1 then error(\"first\") end return { \"text\", \"ok\" } end)")).unwrap();
        assert_eq!(m.render_slot("h", &json!({})), None);
        assert!(m.errors().contains_key("flaky"));
        assert!(m.render_slot("h", &json!({})).is_some());
        assert!(m.errors().is_empty());
    }

    #[test]
    fn failures_are_counted_in_a_row_and_reset_by_a_success() {
        let mut m = manager();
        m.load(plugin("flaky", UI, "local n = 0 discard.slot(\"h\", function() n = n + 1 if n <= 2 or n == 4 then error(\"bad\") end return { \"text\", \"ok\" } end)")).unwrap();
        assert_eq!(m.failure_count("flaky"), 0);
        m.render_slot("h", &json!({}));
        m.render_slot("h", &json!({}));
        assert_eq!(m.failure_count("flaky"), 2);
        m.render_slot("h", &json!({}));
        assert_eq!(m.failure_count("flaky"), 0, "a success resets the count");
        m.render_slot("h", &json!({}));
        assert_eq!(m.failure_count("flaky"), 1);
    }

    #[test]
    fn a_plugin_over_the_threshold_is_reported_as_failing() {
        let mut m = manager();
        m.load(plugin("bad", UI, "discard.slot(\"h\", function() error(\"x\") end)")).unwrap();
        m.load(plugin("good", UI, &text_slot("g", "ok"))).unwrap();
        for _ in 0..3 {
            m.render_slot("h", &json!({}));
        }
        assert_eq!(m.failing(3), ["bad"]);
        assert!(m.failing(4).is_empty());
    }

    #[test]
    fn a_terminated_plugin_counts_as_failing_at_once() {
        let mut m = manager();
        m.load(plugin("loop", UI, "discard.slot(\"s\", function() while true do end end)")).unwrap();
        m.render_slot("s", &json!({}));
        assert_eq!(m.failing(3), ["loop"]);
    }

    #[test]
    fn reloading_a_plugin_resets_its_failures() {
        let mut m = manager();
        m.load(plugin("p", UI, "discard.slot(\"h\", function() error(\"x\") end)")).unwrap();
        for _ in 0..3 {
            m.render_slot("h", &json!({}));
        }
        m.load(plugin("p", UI, &text_slot("h", "fixed"))).unwrap();
        assert_eq!(m.failure_count("p"), 0);
        assert!(m.failing(1).is_empty());
    }

    #[test]
    fn a_terminated_plugin_is_skipped_and_its_theme_is_dropped() {
        let mut m = manager();
        m.load(plugin("themed", UI, "discard.theme({ primary = \"#111111\" }) discard.slot(\"s\", function() while true do end end)")).unwrap();
        assert_eq!(m.theme().color("primary"), Some(Rgb(0x11, 0x11, 0x11)));
        assert_eq!(m.render_slot("s", &json!({})), None);
        assert!(matches!(m.status("themed"), Some(Status::Terminated(_))));
        assert_eq!(m.theme().color("primary"), None);
        assert!(m.errors().contains_key("themed"));
    }

    #[test]
    fn theme_sources_use_last_running_plugin_for_each_token() {
        let mut m = manager();
        m.load(plugin("a", UI, "discard.theme({ primary = \"#111111\", border = \"#222222\" })")).unwrap();
        m.load(plugin("b", UI, "discard.theme({ primary = \"#333333\" }) discard.slot(\"s\", function() while true do end end)")).unwrap();
        assert_eq!(m.theme_sources(), vec![("border".into(), "a".into()), ("primary".into(), "b".into())]);
        m.render_slot("s", &json!({}));
        assert_eq!(m.theme_sources(), vec![("border".into(), "a".into()), ("primary".into(), "a".into())]);
    }

    #[test]
    fn themes_merge_in_load_order() {
        let mut m = manager();
        m.load(plugin("a", UI, "discard.theme({ primary = \"#111111\", border = \"#222222\" })")).unwrap();
        m.load(plugin("b", UI, "discard.theme({ primary = \"#333333\", radius = 4 })")).unwrap();
        let t = m.theme();
        assert_eq!(t.color("primary"), Some(Rgb(0x33, 0x33, 0x33)));
        assert_eq!(t.color("border"), Some(Rgb(0x22, 0x22, 0x22)));
        assert_eq!(t.radius(), Some(4.0));
    }

    #[test]
    fn reloading_a_plugin_replaces_it_in_place() {
        let mut m = manager();
        m.load(plugin("a", UI, &text_slot("h", "v1"))).unwrap();
        m.load(plugin("b", UI, &text_slot("other", "x"))).unwrap();
        m.load(plugin("a", UI, &text_slot("h", "v2"))).unwrap();
        assert_eq!(m.names(), ["a", "b"], "order is kept so precedence does not change");
        assert_eq!(first_text(&m.render_slot("h", &json!({})).unwrap()), "v2");
    }

    #[test]
    fn a_failed_load_changes_nothing_and_keeps_the_old_version_running() {
        let mut m = manager();
        m.load(plugin("a", UI, &text_slot("h", "v1"))).unwrap();
        assert!(m.load(plugin("a", UI, "this is not lua")).is_err());
        assert_eq!(first_text(&m.render_slot("h", &json!({})).unwrap()), "v1");
        assert!(m.load(plugin("new", UI, "this is not lua")).is_err());
        assert_eq!(m.names(), ["a"]);
    }

    #[test]
    fn unloading_removes_the_plugin() {
        let mut m = manager();
        m.load(plugin("a", UI, &text_slot("h", "v1"))).unwrap();
        assert!(m.unload("a"));
        assert!(!m.unload("a"));
        assert_eq!(m.render_slot("h", &json!({})), None);
    }

    #[test]
    fn actions_go_to_the_named_plugin() {
        let mut m = manager();
        m.load(plugin("a", UI, "n = 0 discard.action(\"go\", function() n = n + 1 end) discard.slot(\"s\", function() return { \"text\", n } end)")).unwrap();
        m.load(plugin("b", UI, "discard.action(\"go\", function() error(\"b ran\") end)")).unwrap();
        m.run_action("a", "go").unwrap();
        assert_eq!(first_text(&m.render_slot("s", &json!({})).unwrap()), "1");
        assert!(m.run_action("b", "go").is_err());
        assert_eq!(m.run_action("zzz", "go"), Err(PluginError::NotRegistered { kind: "plugin", name: "zzz".into() }));
    }

    #[test]
    fn commands_run_in_the_first_plugin_that_has_them() {
        let mut m = manager();
        m.load(plugin("a", UI, "discard.command(\"hi\", function() return \"a\" end)")).unwrap();
        m.load(plugin("b", UI, "discard.command(\"hi\", function() return \"b\" end)")).unwrap();
        assert_eq!(m.run_command("hi", "").unwrap().unwrap().as_deref(), Some("a"));
        assert!(m.run_command("nope", "").is_none());
    }

    #[test]
    fn commands_lists_only_running_plugins_in_load_order() {
        let mut m = manager();
        m.load(plugin("b", UI, "discard.command(\"two\", function() end, { description = \"Second\" }) discard.command(\"one\", function() end)")).unwrap();
        m.load(plugin("a", UI, "discard.command(\"spin\", function() while true do end end)")).unwrap();
        let names = |m: &PluginManager| m.commands().iter().map(|(p, c)| (p.clone(), c.name.clone())).collect::<Vec<_>>();
        assert_eq!(names(&m), [("b".to_string(), "one".to_string()), ("b".to_string(), "two".to_string()), ("a".to_string(), "spin".to_string())]);
        assert_eq!(m.commands()[1].1.description.as_deref(), Some("Second"));
        assert!(m.run_command("spin", "").unwrap().is_err());
        assert_eq!(names(&m), [("b".to_string(), "one".to_string()), ("b".to_string(), "two".to_string())]);
    }

    #[test]
    fn a_terminated_plugin_does_not_run_commands() {
        let mut m = manager();
        m.load(plugin("a", UI, "discard.command(\"spin\", function() while true do end end) discard.command(\"hi\", function() return \"a\" end)")).unwrap();
        m.load(plugin("b", UI, "discard.command(\"hi\", function() return \"b\" end)")).unwrap();
        assert!(m.run_command("spin", "").unwrap().is_err());
        assert_eq!(m.run_command("hi", "").unwrap().unwrap().as_deref(), Some("b"));
    }

    #[test]
    fn hooks_collect_one_result_per_plugin_that_handles_the_event() {
        let mut m = manager();
        m.load(plugin("a", UI, "discard.on(\"t\", function() return { block = true, reason = \"a says no\" } end)")).unwrap();
        m.load(plugin("b", UI, "discard.on(\"t\", function() error(\"b broke\") end)")).unwrap();
        m.load(plugin("c", UI, "")).unwrap();
        let results = m.hook("t", &json!({}));
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "a");
        assert!(results[0].1.as_ref().unwrap().block);
        assert!(results[1].1.is_err(), "the caller sees the error and can fail closed");
    }
}
