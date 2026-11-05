//! Cached rendering of the plugin slots.
use super::dto::SlotDto;
use discard_lua::element::{Element, Prop};
use discard_lua::PluginManager;
use std::collections::BTreeMap;

/// Cache the last set of slot trees by the state passed to plugins.
#[derive(Default)]
pub struct SlotCache { key: Option<String>, slots: BTreeMap<String, SlotDto> }
impl SlotCache {
    pub fn clear(&mut self) { self.key = None; self.slots.clear(); }
    pub fn get_or_render(&mut self, state: &serde_json::Value, render: impl FnOnce() -> BTreeMap<String, SlotDto>) -> BTreeMap<String, SlotDto> {
        let key = state.to_string();
        if self.key.as_ref() != Some(&key) { self.slots = render(); self.key = Some(key); }
        self.slots.clone()
    }
}
/// Make a member panel fill the resizable panel.
pub fn fit_member_slot(element: &mut Element) {
    element.props.remove("width");
    element.props.insert("grow".into(), Prop::Bool(true));
}
/// Render the slots used by the main window.
pub fn render_slots(manager: &mut PluginManager, state: &serde_json::Value) -> BTreeMap<String, SlotDto> {
    ["channel_header", "member_list"].into_iter().filter_map(|name| {
        let mut slot = manager.render_slot(name, state)?;
        if name == "member_list" { fit_member_slot(&mut slot.element); }
        Some((name.to_string(), SlotDto { plugin: slot.plugin, element: slot.element }))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use discard_lua::element::{Element, Prop, Tag};
    use std::collections::BTreeMap;

    #[test]
    fn member_slot_root_gets_grow_and_loses_width() {
        let mut element = Element { tag: Tag::Column, props: BTreeMap::from([("width".into(), Prop::Number(200.0))]), children: vec![] };
        fit_member_slot(&mut element);
        assert!(!element.props.contains_key("width"));
        assert_eq!(element.props.get("grow"), Some(&Prop::Bool(true)));
    }
    #[test]
    fn slot_cache_hits_for_identical_state() {
        let mut cache = SlotCache::default();
        let state = serde_json::json!({"a": 1});
        let mut calls = 0;
        cache.get_or_render(&state, || { calls += 1; BTreeMap::new() });
        cache.get_or_render(&state, || { calls += 1; BTreeMap::new() });
        assert_eq!(calls, 1);
        cache.clear();
        cache.get_or_render(&state, || { calls += 1; BTreeMap::new() });
        assert_eq!(calls, 2);
    }
}
