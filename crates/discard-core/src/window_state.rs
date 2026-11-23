//! Window size, position, and panel widths, saved in `$DISCARD_HOME/state.toml`.
//! The app writes this file. `settings.toml` is for the user to edit.

use std::path::{Path, PathBuf};
use std::collections::BTreeMap;

/// The default width of the channel list and the member list.
pub const DEFAULT_PANEL_WIDTH: f32 = 240.0;
pub const MIN_PANEL_WIDTH: f32 = 180.0;
pub const MAX_PANEL_WIDTH: f32 = 400.0;
/// A window smaller than this is not worth restoring.
pub const MIN_WINDOW_WIDTH: f32 = 400.0;
pub const MIN_WINDOW_HEIGHT: f32 = 300.0;
/// At least this much of the window must show on a screen.
pub const MIN_VISIBLE: f32 = 64.0;

/// A rectangle in screen points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Bounds {
    /// The window is big enough, and enough of it shows on one of `screens`.
    pub fn is_restorable(&self, screens: &[Bounds]) -> bool {
        let finite = [self.x, self.y, self.width, self.height]
            .iter()
            .all(|n| n.is_finite());
        finite
            && self.width >= MIN_WINDOW_WIDTH
            && self.height >= MIN_WINDOW_HEIGHT
            && screens.iter().any(|screen| {
                overlap(self.x, self.width, screen.x, screen.width) >= MIN_VISIBLE
                    && overlap(self.y, self.height, screen.y, screen.height) >= MIN_VISIBLE
            })
    }
}

/// The length two ranges share, or 0.
fn overlap(a_start: f32, a_len: f32, b_start: f32, b_len: f32) -> f32 {
    let start = a_start.max(b_start);
    let end = (a_start + a_len).min(b_start + b_len);
    (end - start).max(0.0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelWidths {
    pub channel_list: f32,
    pub member_list: f32,
}

impl Default for PanelWidths {
    fn default() -> Self {
        PanelWidths {
            channel_list: DEFAULT_PANEL_WIDTH,
            member_list: DEFAULT_PANEL_WIDTH,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowState {
    pub selected_conversation: Option<String>,
    /// `None` until a window position was saved.
    pub bounds: Option<Bounds>,
    pub panels: PanelWidths,
    pub member_list_visible: bool,
    pub last_read: BTreeMap<String, String>,
}

impl Default for WindowState {
    fn default() -> Self {
        Self { selected_conversation: None, bounds: None, panels: PanelWidths::default(), member_list_visible: true, last_read: BTreeMap::new() }
    }
}

/// A state file on disk. A missing or bad file gives defaults.
pub struct StateStore {
    path: PathBuf,
    state: WindowState,
}

impl StateStore {
    /// Read the file. Never fails.
    pub fn open(path: &Path) -> StateStore {
        let state = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.parse::<toml::Table>().ok())
            .map(|table| read_state(&table))
            .unwrap_or_default();
        StateStore {
            path: path.to_path_buf(),
            state,
        }
    }

    pub fn state(&self) -> WindowState {
        self.state.clone()
    }

    pub fn set_selected_conversation(&mut self, id: Option<&str>) -> Result<(), String> {
        if self.path.as_os_str().is_empty() { self.state.selected_conversation = id.map(str::to_owned); return Ok(()); }
        self.save(WindowState { selected_conversation: id.map(str::to_owned), ..self.state.clone() })
    }

    pub fn last_read(&self, channel: &str) -> Option<&str> { self.state.last_read.get(channel).map(String::as_str) }

    pub fn member_list_visible(&self) -> bool { self.state.member_list_visible }

    pub fn panels(&self) -> PanelWidths { self.state.panels }

    /// Save the window position and size.
    pub fn set_bounds(&mut self, bounds: Bounds) -> Result<(), String> {
        let all = [bounds.x, bounds.y, bounds.width, bounds.height];
        if !all.iter().all(|n| n.is_finite()) {
            return Err("the window bounds are not finite numbers".to_string());
        }
        self.save(WindowState {
            bounds: Some(bounds),
            ..self.state.clone()
        })
    }

    /// Save the panel widths. Each width is clamped to the allowed range.
    pub fn set_panels(&mut self, panels: PanelWidths) -> Result<(), String> {
        self.save(WindowState {
            panels: PanelWidths {
                channel_list: clamp_width(panels.channel_list),
                member_list: clamp_width(panels.member_list),
            },
            ..self.state.clone()
        })
    }

    /// Save whether the member list is visible.
    pub fn set_member_list_visible(&mut self, visible: bool) -> Result<(), String> {
        self.save(WindowState { member_list_visible: visible, ..self.state.clone() })
    }

    /// Save the last read message for a channel.
    pub fn set_last_read(&mut self, channel: &str, message: &str) -> Result<(), String> {
        let mut next = self.state.clone();
        next.last_read.insert(channel.to_owned(), message.to_owned());
        self.save(next)
    }

    /// Save several read markers with one file write. Each item is `(channel_or_thread_id, message_id)`.
    /// No items means no write.
    pub fn set_last_read_many(&mut self, items: &[(String, String)]) -> Result<(), String> {
        if items.is_empty() { return Ok(()); }
        let mut next = self.state.clone();
        for (channel, message) in items {
            next.last_read.insert(channel.clone(), message.clone());
        }
        self.save(next)
    }

    /// Forget the read marker of a channel that no longer exists. No marker means nothing changes.
    pub fn clear_last_read(&mut self, channel: &str) -> Result<(), String> {
        if !self.state.last_read.contains_key(channel) { return Ok(()); }
        let mut next = self.state.clone();
        next.last_read.remove(channel);
        self.save(next)
    }

    /// Write `next`. On error, the file and the in-memory state stay as they were.
    fn save(&mut self, next: WindowState) -> Result<(), String> {
        let io = |what: &str, e: std::io::Error| format!("{what}: {e}");
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| io("cannot create the directory", e))?;
        }
        let mut tmp_name = self.path.as_os_str().to_owned();
        tmp_name.push(".tmp");
        let tmp = PathBuf::from(tmp_name);
        let result = std::fs::write(&tmp, render(&next))
            .map_err(|e| io("cannot write the state file", e))
            .and_then(|()| {
                std::fs::rename(&tmp, &self.path)
                    .map_err(|e| io("cannot replace the state file", e))
            });
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
            return result;
        }
        self.state = next;
        Ok(())
    }
}

fn clamp_width(width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(MIN_PANEL_WIDTH, MAX_PANEL_WIDTH)
    } else {
        DEFAULT_PANEL_WIDTH
    }
}

fn render(state: &WindowState) -> String {
    let mut text = String::from("# Written by Discard. Safe to delete.\n");
    if let Some(id) = &state.selected_conversation {
        let values: toml::Table = [("selected_conversation".into(), toml::Value::String(id.clone()))].into_iter().collect();
        text.push_str(&toml::to_string(&values).expect("strings serialize"));
    }
    if let Some(b) = state.bounds {
        text.push_str(&format!(
            "\n[window]\nx = {:?}\ny = {:?}\nwidth = {:?}\nheight = {:?}\n",
            b.x, b.y, b.width, b.height
        ));
    }
    text.push_str(&format!(
        "\n[layout]\nchannel_list_width = {:?}\nmember_list_width = {:?}\nmember_list_visible = {}\n",
        state.panels.channel_list, state.panels.member_list, state.member_list_visible
    ));
    if !state.last_read.is_empty() {
        let values: toml::Table = state.last_read.iter().map(|(key, value)| (key.clone(), toml::Value::String(value.clone()))).collect();
        text.push_str("\n[last_read]\n");
        text.push_str(&toml::to_string(&values).expect("strings serialize"));
    }
    text
}

/// A TOML number as `f32`. Rejects `nan` and `inf`.
fn number(value: Option<&toml::Value>) -> Option<f32> {
    let n = match value? {
        toml::Value::Float(f) => *f as f32,
        toml::Value::Integer(i) => *i as f32,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

fn read_state(table: &toml::Table) -> WindowState {
    let bounds = table.get("window").and_then(toml::Value::as_table).and_then(|w| {
        let bounds = Bounds {
            x: number(w.get("x"))?,
            y: number(w.get("y"))?,
            width: number(w.get("width"))?,
            height: number(w.get("height"))?,
        };
        (bounds.width > 0.0 && bounds.height > 0.0).then_some(bounds)
    });
    let layout = table.get("layout").and_then(toml::Value::as_table);
    let width = |key: &str| {
        layout
            .and_then(|l| number(l.get(key)))
            .map_or(DEFAULT_PANEL_WIDTH, clamp_width)
    };
    let last_read = table.get("last_read").and_then(toml::Value::as_table).map(|values| values.iter().filter_map(|(key, value)| value.as_str().filter(|s| !s.is_empty()).map(|s| (key.clone(), s.to_owned()))).collect()).unwrap_or_default();
    WindowState {
        selected_conversation: table.get("selected_conversation").and_then(toml::Value::as_str).map(str::to_owned),
        last_read,
        bounds,
        member_list_visible: layout.and_then(|l| l.get("member_list_visible")).and_then(toml::Value::as_bool).unwrap_or(true),
        panels: PanelWidths {
            channel_list: width("channel_list_width"),
            member_list: width("member_list_width"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn path_in(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("state.toml")
    }

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds {
        Bounds { x, y, width, height }
    }

    #[test]
    fn a_missing_file_gives_defaults_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(&path_in(&dir));
        assert_eq!(store.state(), WindowState::default());
        assert_eq!(store.state().panels.channel_list, 240.0);
        assert_eq!(store.state().bounds, None);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn member_list_visibility_persists_and_bad_values_default_to_visible() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let mut store = StateStore::open(&path);
        assert!(store.state().member_list_visible);
        store.set_panels(PanelWidths { channel_list: 300.0, member_list: 366.0 }).unwrap();
        store.set_member_list_visible(false).unwrap();
        let hidden = StateStore::open(&path).state();
        assert!(!hidden.member_list_visible);
        assert_eq!(hidden.panels, PanelWidths { channel_list: 300.0, member_list: 366.0 });
        store.set_member_list_visible(true).unwrap();
        let shown = StateStore::open(&path).state();
        assert!(shown.member_list_visible);
        assert_eq!(shown.panels, hidden.panels);
        fs::write(&path, "[layout]\nmember_list_visible = \"no\"\n").unwrap();
        assert!(StateStore::open(&path).state().member_list_visible);
    }

    #[test]
    fn accessors_return_current_values() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = StateStore::open(&path_in(&dir));
        assert_eq!(store.last_read("general"), None);
        assert!(store.member_list_visible());
        store.set_last_read("general", "one").unwrap();
        store.set_member_list_visible(false).unwrap();
        assert_eq!(store.last_read("general"), Some("one"));
        assert!(!store.member_list_visible());
    }

    #[test]
    fn read_markers_persist_and_bad_values_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let mut store = StateStore::open(&path);
        store.set_last_read("general", "message-1").unwrap();
        assert_eq!(StateStore::open(&path).state().last_read.get("general").map(String::as_str), Some("message-1"));
        fs::write(&path, "[last_read]\ngeneral = 42\n").unwrap();
        assert!(StateStore::open(&path).state().last_read.is_empty());
    }

    #[test]
    fn a_cleared_read_marker_stays_cleared_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let mut store = StateStore::open(&path);
        store.set_last_read("a", "m1").unwrap();
        store.set_last_read("b", "m2").unwrap();
        store.clear_last_read("a").unwrap();
        store.clear_last_read("missing").unwrap();
        assert_eq!((store.last_read("a"), store.last_read("b")), (None, Some("m2")));
        let reopened = StateStore::open(&path);
        assert_eq!((reopened.last_read("a"), reopened.last_read("b")), (None, Some("m2")));
    }

    #[test]
    fn saved_values_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let mut store = StateStore::open(&path);
        store.set_bounds(bounds(10.0, 20.0, 1200.0, 800.0)).unwrap();
        store
            .set_panels(PanelWidths { channel_list: 300.0, member_list: 200.0 })
            .unwrap();
        let reopened = StateStore::open(&path);
        assert_eq!(reopened.state(), store.state());
        assert_eq!(reopened.state().bounds, Some(bounds(10.0, 20.0, 1200.0, 800.0)));
        assert_eq!(reopened.state().panels.channel_list, 300.0);
        assert_eq!(reopened.state().panels.member_list, 200.0);
    }

    #[test]
    fn saving_bounds_keeps_the_panel_widths_and_the_reverse() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let mut store = StateStore::open(&path);
        store.set_panels(PanelWidths { channel_list: 300.0, member_list: 200.0 }).unwrap();
        store.set_bounds(bounds(0.0, 0.0, 900.0, 700.0)).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[window]") && text.contains("[layout]"), "{text}");
        assert_eq!(StateStore::open(&path).state().panels.channel_list, 300.0);
    }

    #[test]
    fn panel_widths_are_clamped_when_saved_and_when_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let mut store = StateStore::open(&path);
        store.set_panels(PanelWidths { channel_list: 10.0, member_list: 9000.0 }).unwrap();
        assert_eq!(store.state().panels.channel_list, MIN_PANEL_WIDTH);
        assert_eq!(store.state().panels.member_list, MAX_PANEL_WIDTH);

        fs::write(&path, "[layout]\nchannel_list_width = 5\nmember_list_width = 5000\n").unwrap();
        let read = StateStore::open(&path).state();
        assert_eq!(read.panels.channel_list, MIN_PANEL_WIDTH);
        assert_eq!(read.panels.member_list, MAX_PANEL_WIDTH);
    }

    #[test]
    fn bad_values_fall_back_one_by_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(
            &path,
            "[window]\nx = \"left\"\ny = 1\nwidth = 800\nheight = 600\n[layout]\nchannel_list_width = \"wide\"\nmember_list_width = 260\n",
        )
        .unwrap();
        let state = StateStore::open(&path).state();
        assert_eq!(state.bounds, None, "a window with a bad number is dropped");
        assert_eq!(state.panels.channel_list, DEFAULT_PANEL_WIDTH);
        assert_eq!(state.panels.member_list, 260.0);
    }

    #[test]
    fn a_non_finite_or_nonsense_window_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(&path, "[window]\nx = 0\ny = 0\nwidth = -5\nheight = 600\n").unwrap();
        assert_eq!(StateStore::open(&path).state().bounds, None);
        fs::write(&path, "[window]\nx = nan\ny = 0\nwidth = 800\nheight = 600\n").unwrap();
        assert_eq!(StateStore::open(&path).state().bounds, None);
    }

    #[test]
    fn an_unparseable_file_gives_defaults_and_the_next_save_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        fs::write(&path, "this is = [not toml").unwrap();
        let mut store = StateStore::open(&path);
        assert_eq!(store.state(), WindowState::default());
        store.set_bounds(bounds(1.0, 2.0, 800.0, 600.0)).unwrap();
        assert_eq!(StateStore::open(&path).state().bounds, Some(bounds(1.0, 2.0, 800.0, 600.0)));
    }

    #[test]
    fn a_save_fails_cleanly_when_the_directory_is_missing_and_cannot_be_made() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        fs::write(&blocker, "x").unwrap();
        let mut store = StateStore::open(&blocker.join("state.toml"));
        assert!(store.set_bounds(bounds(0.0, 0.0, 800.0, 600.0)).is_err());
    }

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
    }

    #[test]
    fn set_last_read_many_saves_every_marker_in_one_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let mut store = StateStore::open(&path);
        store.set_last_read_many(&pairs(&[("a", "m1"), ("b", "m2")])).unwrap();
        assert_eq!((store.last_read("a"), store.last_read("b")), (Some("m1"), Some("m2")));
        let reopened = StateStore::open(&path);
        assert_eq!((reopened.last_read("a"), reopened.last_read("b")), (Some("m1"), Some("m2")));
    }

    #[test]
    fn set_last_read_many_with_no_items_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir);
        let mut store = StateStore::open(&path);
        store.set_last_read_many(&[]).unwrap();
        assert!(!path.exists());
        store.set_last_read("a", "m1").unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        store.set_last_read_many(&[]).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn a_failed_set_last_read_many_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = StateStore::open(&path_in(&dir));
        store.set_last_read("a", "old-a").unwrap();
        store.set_last_read("b", "old-b").unwrap();
        let blocker = dir.path().join("file");
        fs::write(&blocker, "x").unwrap();
        store.path = blocker.join("state.toml");
        assert!(store.set_last_read_many(&pairs(&[("a", "new-a"), ("b", "new-b")])).is_err());
        assert_eq!((store.last_read("a"), store.last_read("b")), (Some("old-a"), Some("old-b")));
    }

    #[test]
    fn restorable_needs_a_big_enough_window_that_shows_on_a_screen() {
        let screens = [bounds(0.0, 0.0, 1920.0, 1080.0), bounds(1920.0, 0.0, 1280.0, 720.0)];
        assert!(bounds(100.0, 100.0, 1200.0, 800.0).is_restorable(&screens));
        assert!(bounds(2000.0, 50.0, 800.0, 600.0).is_restorable(&screens), "second screen");
        assert!(bounds(-300.0, 100.0, 800.0, 600.0).is_restorable(&screens), "partly off");
        assert!(!bounds(5000.0, 100.0, 800.0, 600.0).is_restorable(&screens), "off every screen");
        assert!(!bounds(1900.0 - 20.0, 1070.0, 800.0, 600.0).is_restorable(&screens), "a sliver shows");
        assert!(!bounds(0.0, 0.0, 100.0, 100.0).is_restorable(&screens), "too small");
        assert!(!bounds(0.0, 0.0, 800.0, 600.0).is_restorable(&[]), "no screens");
    }
}
