//! The identity of an app window's view.
use discard_core::model::ProjectId;

const MAIN: &str = "main";
const PROJECT_PREFIX: &str = "project-";

/// Names the view of one app window. The ID is the label of the window.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewId(String);
impl ViewId {
    pub fn main() -> ViewId { ViewId(MAIN.into()) }
    /// The view of a project window. A character of the project ID that a window label does not allow becomes
    /// `_` and two hex digits for each byte. `_` itself is escaped, so the label decodes to the original ID.
    pub fn project(id: &ProjectId) -> ViewId {
        let mut label = String::from(PROJECT_PREFIX);
        for byte in id.0.bytes() {
            if byte.is_ascii_alphanumeric() || b"-/:".contains(&byte) { label.push(byte as char); } else { label.push_str(&format!("_{byte:02x}")); }
        }
        ViewId(label)
    }
    /// The view of the window with this label. `None` when the label is not the label of an app window.
    pub fn from_label(label: &str) -> Option<ViewId> {
        if label == MAIN { return Some(ViewId::main()); }
        let id = decode(label.strip_prefix(PROJECT_PREFIX)?).filter(|id| !id.is_empty())?;
        // A label that does not match its own encoding is not the label of an app window.
        Some(ViewId::project(&ProjectId(id))).filter(|view| view.0 == label)
    }
    pub fn label(&self) -> &str { &self.0 }
    pub fn is_main(&self) -> bool { self.0 == MAIN }
    pub fn project_id(&self) -> Option<ProjectId> { decode(self.0.strip_prefix(PROJECT_PREFIX)?).map(ProjectId) }
}
fn decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.bytes();
    while let Some(byte) = rest.next() {
        if byte != b'_' { bytes.push(byte); continue; }
        let (high, low) = (rest.next()?, rest.next()?);
        bytes.push(u8::from_str_radix(std::str::from_utf8(&[high, low]).ok()?, 16).ok()?);
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_round_trip() {
        let id = ProjectId("0193a1b2-aaaa-7bbb-8ccc-0123456789ab".into());
        let view = ViewId::project(&id);
        assert_eq!(view.label(), "project-0193a1b2-aaaa-7bbb-8ccc-0123456789ab");
        assert_eq!(ViewId::from_label(view.label()), Some(view.clone()));
        assert_eq!(view.project_id(), Some(id));
        assert!(ViewId::from_label("main").unwrap().is_main());
        assert_eq!(ViewId::from_label("settings"), None);
    }
    #[test]
    fn a_project_label_is_a_valid_tauri_label() {
        let view = ViewId::project(&ProjectId("a b/c".into()));
        assert!(view.label().chars().all(|c| c.is_ascii_alphanumeric() || "-/:_".contains(c)));
        assert_eq!(view.project_id(), Some(ProjectId("a b/c".into())));
    }
    #[test]
    fn escapes_do_not_collide_and_a_foreign_label_is_not_a_view() {
        for raw in ["a_b", "a b", "_", "é", "a_20b"] {
            let view = ViewId::project(&ProjectId(raw.into()));
            assert_eq!(view.project_id(), Some(ProjectId(raw.into())), "{raw}");
            assert_eq!(ViewId::from_label(view.label()), Some(view), "{raw}");
        }
        assert_ne!(ViewId::project(&ProjectId("a b".into())), ViewId::project(&ProjectId("a_20b".into())));
        for label in ["", "project-", "project-a b", "project-a_zz", "project-a_2", "project-_41", "Main", "project"] {
            assert_eq!(ViewId::from_label(label), None, "{label:?}");
        }
        assert_eq!(ViewId::main().project_id(), None);
    }
}
