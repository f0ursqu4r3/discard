//! State both windows share. Stored as a GPUI global so `open_settings` can reach it.

use discard_core::settings::SettingsStore;
use discard_lua::{LoadReport, PluginService};
use gpui_kit::Global;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Shared {
    pub plugins: Arc<PluginService>,
    pub load_report: Rc<LoadReport>,
    pub settings: Arc<Mutex<SettingsStore>>,
    /// Task 6 bumps this when plugin state changes.
    pub plugin_epoch: Arc<AtomicU64>,
    /// Task 6 shows this notice on the Plugins page.
    pub plugin_notice: Arc<Mutex<Option<String>>>,
}

impl Global for Shared {}
