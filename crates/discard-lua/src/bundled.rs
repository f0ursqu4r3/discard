//! Plugins that ship inside the Discard binary.

use crate::snapshot::Snapshot;

/// The default UI, written in Lua like any other plugin.
pub fn default_ui() -> Snapshot {
    Snapshot::from_files(
        [
            ("plugin.toml".to_string(), include_bytes!("../bundled/default-ui/plugin.toml").to_vec()),
            ("init.lua".to_string(), include_bytes!("../bundled/default-ui/init.lua").to_vec()),
        ]
        .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{Node, Prop, Tag};
    use crate::manager::PluginManager;
    use crate::runtime::PluginLimits;
    use serde_json::json;

    fn state(pending: u64, status: &str) -> serde_json::Value {
        json!({
            "project": "demo", "channel": "general", "pending_approvals": pending,
            "agent": { "id": "agent-claude", "name": "Claude", "provider": "claude", "status": status },
        })
    }

    fn manager() -> PluginManager {
        let mut m = PluginManager::new(PluginLimits::default());
        m.load(default_ui()).expect("the bundled default UI must load");
        m
    }

    fn texts(e: &crate::element::Element, out: &mut Vec<String>) {
        for c in &e.children {
            match c {
                Node::Text(t) => out.push(t.clone()),
                Node::Element(inner) => texts(inner, out),
            }
        }
    }

    #[test]
    fn the_default_ui_is_a_valid_ui_only_plugin_with_two_slots() {
        let m = manager();
        let info = m.info("default-ui").unwrap();
        assert!(info.manifest.capabilities.is_ui_only());
        assert_eq!(info.slots, ["channel_header", "member_list"]);
    }

    #[test]
    fn the_header_shows_the_channel_and_hides_the_badge_without_pending_approvals() {
        let mut m = manager();
        let r = m.render_slot("channel_header", &state(0, "idle")).unwrap();
        let mut t = Vec::new();
        texts(&r.element, &mut t);
        assert_eq!(t, ["# general"]);
        let Node::Element(name) = &r.element.children[0] else { panic!() };
        assert_eq!(name.props.get("variant"), Some(&Prop::Text("mono".into())));
    }

    #[test]
    fn the_header_shows_a_badge_and_a_warning_when_approvals_are_pending() {
        let mut m = manager();
        let r = m.render_slot("channel_header", &state(3, "awaiting_approval")).unwrap();
        let mut t = Vec::new();
        texts(&r.element, &mut t);
        assert_eq!(t, ["# general", "Needs approval"]);
        let Node::Element(group) = r.element.children.last().unwrap() else { panic!() };
        let Node::Element(badge) = &group.children[0] else { panic!() };
        assert_eq!((badge.tag, &badge.props["count"]), (Tag::Badge, &Prop::Number(3.0)));
    }

    #[test]
    fn the_member_list_shows_agent_identity_and_a_spinner_or_status_dot() {
        let mut m = manager();
        for (status, label, color) in [
            ("working", "Working · claude", "success"),
            ("awaiting_approval", "Waiting for approval · claude", "warning"),
            ("idle", "Idle · claude", "muted_foreground"),
            ("offline", "Offline · claude", "border"),
            ("unknown", "unknown · claude", "border"),
        ] {
            let r = m.render_slot("member_list", &state(0, status)).unwrap();
            let mut t = Vec::new();
            texts(&r.element, &mut t);
            assert!(t.contains(&"Claude".to_string()) && t.contains(&label.to_string()), "{t:?}");
            assert_eq!(r.element.props.get("agent"), Some(&Prop::Text("agent-claude".into())));
            let Node::Element(heading) = &r.element.children[0] else { panic!() };
            assert_eq!(heading.children, [Node::Text("Agents".into())]);
            assert_eq!(heading.props.get("variant"), Some(&Prop::Text("label".into())));
            let Node::Element(row) = &r.element.children[1] else { panic!() };
            let Node::Element(avatar) = &row.children[0] else { panic!() };
            assert_eq!(avatar.tag, Tag::Avatar);
            assert_eq!(avatar.props.get("agent"), Some(&Prop::Text("agent-claude".into())));
            if status == "working" {
                assert_eq!(avatar.props.get("status"), None);
                assert_eq!(row.children.len(), 3);
                let Node::Element(spinner) = &row.children[2] else { panic!() };
                assert_eq!(spinner.tag, Tag::Spinner);
            } else {
                assert_eq!(avatar.props.get("status"), Some(&Prop::Text(color.into())));
                assert_eq!(row.children.len(), 2);
            }
            let Node::Element(column) = &row.children[1] else { panic!() };
            assert_eq!(column.props.get("grow"), Some(&Prop::Bool(true)));
            let Node::Element(name) = &column.children[0] else { panic!() };
            assert_eq!(name.props.get("color"), Some(&Prop::Text("agent".into())));
            assert_eq!(name.props.get("weight"), Some(&Prop::Text("semibold".into())));
            let Node::Element(line) = &column.children[1] else { panic!() };
            assert!(line.children.iter().all(|node| !matches!(node, Node::Element(e) if e.tag == Tag::Dot)));
        }
    }

    #[test]
    fn a_user_plugin_can_replace_a_default_slot() {
        let mut m = manager();
        let custom = Snapshot::from_files(
            [
                ("plugin.toml".to_string(), b"name = \"mine\"\nversion = \"1\"\napi = \"1\"\n[capabilities]\nui = true\n".to_vec()),
                ("init.lua".to_string(), b"discard.slot(\"channel_header\", function() return { \"text\", \"custom\" } end)".to_vec()),
            ]
            .into(),
        );
        m.load(custom).unwrap();
        assert_eq!(m.render_slot("channel_header", &state(0, "idle")).unwrap().plugin, "mine");
        assert_eq!(m.render_slot("member_list", &state(0, "idle")).unwrap().plugin, "default-ui");
    }
}
