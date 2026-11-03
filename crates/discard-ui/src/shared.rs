//! State both windows share. Stored as a GPUI global so `open_settings` can reach it.

use discard_core::settings::SettingsStore;
use discard_lua::theme::{ProtectedPalette, Rgb};
use discard_lua::{LoadReport, PluginService};
use gpui_kit::{Global, Hsla, Rgba};
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Shared {
    pub plugins: Arc<PluginService>,
    pub load_report: Rc<LoadReport>,
    pub settings: Arc<Mutex<SettingsStore>>,
    /// Bumped when plugin state changes (approve, reject, roll back, revert). Each window compares it to redraw.
    pub plugin_epoch: Arc<AtomicU64>,
    /// The last plugin notice or error. The Plugins page shows it, and the main window counts it as a problem.
    pub plugin_notice: Arc<Mutex<Option<String>>>,
}

impl Global for Shared {}

/// Colors for protected surfaces when the theme says nothing.
pub(crate) const PROTECTED_DEFAULT: ProtectedPalette = ProtectedPalette {
    background: Rgb(0x1e, 0x1e, 0x1e),
    foreground: Rgb(0xf0, 0xf0, 0xf0),
    border: Rgb(0xff, 0xaa, 0x00),
};

pub(crate) fn hsla(c: Rgb) -> Hsla {
    Rgba {
        r: f32::from(c.0) / 255.0,
        g: f32::from(c.1) / 255.0,
        b: f32::from(c.2) / 255.0,
        a: 1.0,
    }
    .into()
}
