//! Delivers engine events to plugins. A single thread keeps the events in order, so the engine never waits for a
//! plugin or for the plugin manager lock (the UI holds that lock while it renders).

use discard_core::observer::EventObserver;
use discard_lua::PluginManager;
use serde_json::Value;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

pub struct PluginEvents {
    tx: Sender<(String, Value)>,
}

impl PluginEvents {
    pub fn new(manager: Arc<Mutex<PluginManager>>) -> Self {
        let (tx, rx) = channel::<(String, Value)>();
        std::thread::Builder::new()
            .name("plugin-events".into())
            .spawn(move || {
                // Ends when the last sender is dropped.
                for (event, payload) in rx {
                    manager.lock().unwrap_or_else(|p| p.into_inner()).emit(&event, &payload);
                }
            })
            .expect("could not start the plugin event thread");
        Self { tx }
    }
}

impl EventObserver for PluginEvents {
    fn observe(&self, event: &str, payload: &Value) {
        // The thread only stops when this struct is dropped, so a send error cannot happen in practice.
        let _ = self.tx.send((event.to_string(), payload.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_lua::PluginLimits;
    use discard_lua::snapshot::Snapshot;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn manager_with(init: &str) -> Arc<Mutex<PluginManager>> {
        let manifest = "name = \"t\"\nversion = \"1\"\napi = \"1\"\n[capabilities]\nui = true\n";
        let snapshot = Snapshot::from_files(
            [("plugin.toml".to_string(), manifest.as_bytes().to_vec()), ("init.lua".to_string(), init.as_bytes().to_vec())].into(),
        );
        let mut m = PluginManager::new(PluginLimits::default());
        m.load(snapshot).unwrap();
        Arc::new(Mutex::new(m))
    }

    fn command(m: &Arc<Mutex<PluginManager>>, name: &str) -> String {
        m.lock().unwrap().run_command(name, "").unwrap().unwrap().unwrap()
    }

    fn wait_for(m: &Arc<Mutex<PluginManager>>, expected: &str) -> String {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let got = command(m, "log");
            if got == expected || Instant::now() > end {
                return got;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    const RECORD: &str = r#"local log = {}
        discard.on("message_sent", function(e) log[#log + 1] = "m:" .. e.text end)
        discard.on("tool_call", function(e) log[#log + 1] = "t:" .. e.tool end)
        discard.command("log", function() return table.concat(log, ",") end)"#;

    #[test]
    fn plugins_receive_events_in_the_order_the_engine_sent_them() {
        let manager = manager_with(RECORD);
        let events = PluginEvents::new(Arc::clone(&manager));
        events.observe("message_sent", &json!({"text": "one"}));
        events.observe("tool_call", &json!({"tool": "Bash"}));
        events.observe("message_sent", &json!({"text": "two"}));
        assert_eq!(wait_for(&manager, "m:one,t:Bash,m:two"), "m:one,t:Bash,m:two");
    }

    #[test]
    fn an_event_no_plugin_subscribed_to_is_ignored() {
        let manager = manager_with(RECORD);
        let events = PluginEvents::new(Arc::clone(&manager));
        events.observe("turn_finished", &json!({"outcome": "completed"}));
        events.observe("message_sent", &json!({"text": "after"}));
        assert_eq!(wait_for(&manager, "m:after"), "m:after");
    }

    #[test]
    fn the_engine_side_does_not_wait_while_the_manager_is_locked() {
        let manager = manager_with(RECORD);
        let events = PluginEvents::new(Arc::clone(&manager));
        let guard = manager.lock().unwrap();
        let started = Instant::now();
        events.observe("message_sent", &json!({"text": "queued"}));
        assert!(started.elapsed() < Duration::from_millis(200), "observe must return while the lock is held");
        drop(guard);
        assert_eq!(wait_for(&manager, "m:queued"), "m:queued");
    }
}
