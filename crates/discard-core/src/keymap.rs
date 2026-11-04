//! Keybinding parsing, validation, and conflict resolution.
use crate::{
    action::{Action, default_keymap},
    settings::SettingsIssue,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord(String);
impl Chord {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRule {
    Locked,
    Required,
    Optional,
}
pub fn rule(action: Action) -> Option<KeyRule> {
    if action.needs_target() {
        return None;
    }
    Some(match action {
        Action::CloseWindow | Action::HideApp | Action::Minimize => KeyRule::Locked,
        Action::Quit | Action::OpenSettings | Action::CancelTurn => KeyRule::Required,
        _ => KeyRule::Optional,
    })
}
pub const RESERVED: &[(&str, Option<Action>)] = &[
    ("cmd-q", Some(Action::Quit)),
    ("cmd-,", Some(Action::OpenSettings)),
    ("cmd-w", Some(Action::CloseWindow)),
    ("cmd-h", Some(Action::HideApp)),
    ("cmd-m", Some(Action::Minimize)),
    ("cmd-c", None),
    ("cmd-v", None),
    ("cmd-x", None),
    ("cmd-a", None),
    ("cmd-z", None),
];
const NAMED_KEYS: &[&str] = &[
    "enter",
    "escape",
    "tab",
    "space",
    "backspace",
    "delete",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
    "insert",
];
#[derive(Debug, thiserror::Error)]
pub enum KeymapError {
    #[error("Invalid shortcut: {0}")]
    BadChord(String),
    #[error("{} cannot have a shortcut", .0.label())]
    NotBindable(Action),
    #[error("{} is locked", .0.label())]
    Locked(Action),
    #[error("{} cannot be unbound", .0.label())]
    CannotUnbind(Action),
    #[error("{} is reserved{}", display_chord(&Chord(chord.clone())), owner.map(|a| format!(" for {}", a.label())).unwrap_or_default())]
    Reserved {
        chord: String,
        owner: Option<Action>,
    },
    #[error("{} is already used by {}", display_chord(&Chord(chord.clone())), action.label())]
    InUse { chord: String, action: Action },
}
pub fn parse_chord(text: &str) -> Result<Chord, KeymapError> {
    let lower = text.to_lowercase();
    let (prefix, key) = if let Some(prefix) = lower.strip_suffix("--") {
        (prefix, "-")
    } else {
        match lower.rsplit_once('-') {
            Some((prefix, key)) => (prefix, key),
            None => ("", lower.as_str()),
        }
    };
    let mut mods = [false; 4];
    if !prefix.is_empty() {
        for part in prefix.split('-') {
            let idx = match part {
                "ctrl" => 0,
                "alt" => 1,
                "shift" => 2,
                "cmd" => 3,
                _ => return Err(KeymapError::BadChord(text.into())),
            };
            if mods[idx] {
                return Err(KeymapError::BadChord(text.into()));
            }
            mods[idx] = true;
        }
    }
    let function = key
        .strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=12).contains(&n) && key == format!("f{n}"));
    let printable =
        key.chars().count() == 1 && key.chars().all(|c| !c.is_control() && !c.is_whitespace());
    if !(printable || function || NAMED_KEYS.contains(&key))
        || !(mods[0] || mods[1] || mods[3] || function)
    {
        return Err(KeymapError::BadChord(text.into()));
    }
    let mut parts: Vec<&str> = ["ctrl", "alt", "shift", "cmd"]
        .into_iter()
        .zip(mods)
        .filter_map(|(name, on)| on.then_some(name))
        .collect();
    parts.push(key);
    Ok(Chord(parts.join("-")))
}
pub fn display_chord(chord: &Chord) -> String {
    let s = chord.as_str();
    let (prefix, key) = if let Some(p) = s.strip_suffix("--") {
        (p, "-")
    } else {
        s.rsplit_once('-').unwrap_or(("", s))
    };
    let mut parts: Vec<String> = if prefix.is_empty() {
        Vec::new()
    } else {
        prefix
            .split('-')
            .map(|p| {
                match p {
                    "ctrl" => "Ctrl",
                    "alt" => "Alt",
                    "shift" => "Shift",
                    "cmd" => "Cmd",
                    _ => p,
                }
                .to_string()
            })
            .collect()
    };
    if let Some(index) = parts.iter().position(|p| p == "Cmd") {
        let cmd = parts.remove(index);
        parts.insert(0, cmd);
    }
    parts.push(key.to_uppercase());
    parts.join("+")
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    entries: Vec<(Action, Option<Chord>, bool)>,
}
impl Default for Keymap {
    fn default() -> Self {
        Self::defaults()
    }
}
impl Keymap {
    pub fn defaults() -> Self {
        Self {
            entries: Action::all()
                .iter()
                .copied()
                .filter(|a| rule(*a).is_some())
                .map(|a| {
                    (
                        a,
                        default_keymap()
                            .iter()
                            .find(|(action, _)| *action == a)
                            .map(|(_, s)| Chord((*s).into())),
                        false,
                    )
                })
                .collect(),
        }
    }
    pub fn chord(&self, action: Action) -> Option<&Chord> {
        self.entries
            .iter()
            .find(|(a, _, _)| *a == action)
            .and_then(|(_, c, _)| c.as_ref())
    }
    pub fn is_custom(&self, action: Action) -> bool {
        self.entries
            .iter()
            .any(|(a, _, custom)| *a == action && *custom)
    }
    pub fn bindings(&self) -> Vec<(Action, Chord)> {
        self.entries
            .iter()
            .filter_map(|(a, c, _)| c.clone().map(|c| (*a, c)))
            .collect()
    }
}
pub fn check_change(
    map: &Keymap,
    action: Action,
    chord: Option<&str>,
) -> Result<Option<Chord>, KeymapError> {
    match rule(action) {
        None => return Err(KeymapError::NotBindable(action)),
        Some(KeyRule::Locked) => return Err(KeymapError::Locked(action)),
        Some(KeyRule::Required) if chord.is_none_or(str::is_empty) => {
            return Err(KeymapError::CannotUnbind(action));
        }
        _ => {}
    }
    let Some(text) = chord.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let chord = parse_chord(text)?;
    if let Some((_, owner)) = RESERVED
        .iter()
        .find(|(s, owner)| *s == chord.as_str() && *owner != Some(action))
    {
        return Err(KeymapError::Reserved {
            chord: chord.0,
            owner: *owner,
        });
    }
    if let Some((other, _)) = map
        .bindings()
        .into_iter()
        .find(|(a, c)| *a != action && *c == chord)
    {
        return Err(KeymapError::InUse {
            chord: chord.0,
            action: other,
        });
    }
    Ok(Some(chord))
}
pub fn resolve(raw: &[(String, String)]) -> (Keymap, Vec<SettingsIssue>) {
    let mut map;
    let mut issues = Vec::new();
    let mut candidates = Vec::new();
    for (id, text) in raw {
        let result = match Action::from_id(id) {
            None => Err(format!("unknown action `{id}`")),
            Some(a) => {
                let empty = Keymap {
                    entries: Vec::new(),
                };
                check_change(&empty, a, Some(text))
                    .map(|c| (a, c))
                    .map_err(|e| e.to_string())
            }
        };
        match result {
            Ok(candidate) => candidates.push((id, candidate)),
            Err(message) => issues.push(SettingsIssue {
                key: format!("keybindings.{id}"),
                message,
            }),
        }
    }
    candidates.sort_by_key(|(_, (a, _))| {
        Action::all()
            .iter()
            .position(|v| v == a)
            .unwrap_or(usize::MAX)
    });
    loop {
        map = Keymap::defaults();
        for (_, (a, _)) in &candidates {
            if let Some(entry) = map.entries.iter_mut().find(|e| e.0 == *a) {
                entry.1 = None;
            }
        }
        let mut rejected = None;
        for (index, (id, (a, c))) in candidates.iter().enumerate() {
            if let Some(other) = c.as_ref().and_then(|ch| {
                map.bindings()
                    .into_iter()
                    .find(|(other, value)| other != a && value == ch)
                    .map(|(other, _)| other)
            }) {
                issues.push(SettingsIssue {
                    key: format!("keybindings.{id}"),
                    message: KeymapError::InUse {
                        chord: c.as_ref().unwrap().0.clone(),
                        action: other,
                    }
                    .to_string(),
                });
                rejected = Some(index);
                break;
            }
            if let Some(entry) = map.entries.iter_mut().find(|e| e.0 == *a) {
                entry.1 = c.clone();
                entry.2 = true;
            }
        }
        match rejected {
            Some(index) => {
                candidates.remove(index);
            }
            None => break,
        }
    }
    (map, issues)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    pub action: Action,
    pub rule: KeyRule,
    pub chord: Option<Chord>,
    pub default: Option<Chord>,
    pub custom: bool,
}
pub fn rows(map: &Keymap) -> Vec<KeyRow> {
    let defaults = Keymap::defaults();
    Action::all()
        .iter()
        .filter_map(|&action| {
            rule(action).map(|rule| KeyRow {
                action,
                rule,
                chord: map.chord(action).cloned(),
                default: defaults.chord(action).cloned(),
                custom: map.is_custom(action),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap;

    fn chord(text: &str) -> Chord {
        keymap::parse_chord(text).unwrap()
    }
    #[test]
    fn parse_chord_puts_modifiers_in_canonical_order_and_lowercases() {
        assert_eq!(chord("Cmd-Shift-P").as_str(), "shift-cmd-p");
        assert_eq!(chord("cmd-alt-ctrl-k").as_str(), "ctrl-alt-cmd-k");
    }
    #[test]
    fn parse_chord_accepts_a_comma_and_a_dash_as_the_key() {
        assert_eq!(chord("cmd-,").as_str(), "cmd-,");
        assert_eq!(chord("cmd--").as_str(), "cmd--");
    }
    #[test]
    fn parse_chord_rejects() {
        for text in [
            "",
            "cmd-",
            "p",
            "shift-p",
            "cmd-pp",
            "cmd-cmd-p",
            "hyper-p",
            "cmd-f13",
        ] {
            assert!(keymap::parse_chord(text).is_err(), "{text}");
        }
        assert!(keymap::parse_chord("f5").is_ok());
    }
    #[test]
    fn display_chord_joins_capitalized_parts_with_plus() {
        assert_eq!(keymap::display_chord(&chord("cmd-shift-p")), "Cmd+Shift+P");
    }
    #[test]
    fn every_action_has_a_rule_and_the_copy_actions_have_none() {
        for &a in Action::all() {
            assert_eq!(keymap::rule(a).is_none(), a.needs_target());
        }
    }
    #[test]
    fn defaults_match_default_keymap_and_reserved_owners() {
        let map = Keymap::defaults();
        for &(a, s) in default_keymap() {
            assert_eq!(map.chord(a), Some(&chord(s)));
        }
        for &(s, owner) in keymap::RESERVED {
            if let Some(a) = owner {
                assert_eq!(map.chord(a), Some(&chord(s)));
            }
        }
    }
    #[test]
    fn resolve_keeps_a_valid_override_and_marks_it_custom() {
        let (map, issues) = keymap::resolve(&[("zoom".into(), "cmd-shift-z".into())]);
        assert!(issues.is_empty());
        assert_eq!(map.chord(Action::Zoom), Some(&chord("cmd-shift-z")));
        assert!(map.is_custom(Action::Zoom));
    }
    #[test]
    fn resolve_drops_each_bad_entry_with_one_issue() {
        for (id, value) in [
            ("unknown", "cmd-k"),
            ("zoom", "p"),
            ("hide_app", "cmd-k"),
            ("zoom", "cmd-q"),
            ("quit", ""),
        ] {
            let (map, issues) = keymap::resolve(&[(id.into(), value.into())]);
            assert_eq!(issues.len(), 1, "{id}");
            assert_eq!(issues[0].key, format!("keybindings.{id}"));
            assert_eq!(map, Keymap::defaults());
        }
    }
    #[test]
    fn resolve_lets_a_default_chord_beat_a_custom_one() {
        let (map, issues) = keymap::resolve(&[("zoom".into(), "cmd-.".into())]);
        assert_eq!(issues.len(), 1);
        assert_eq!(map, Keymap::defaults());
    }
    #[test]
    fn resolve_lets_the_first_action_win_between_two_custom_chords() {
        let (map, issues) = keymap::resolve(&[
            ("zoom".into(), "cmd-k".into()),
            ("review_plugins".into(), "cmd-k".into()),
        ]);
        assert_eq!(issues.len(), 1);
        assert_eq!(map.chord(Action::ReviewPlugins), Some(&chord("cmd-k")));
        assert_eq!(map.chord(Action::Zoom), None);
    }
    #[test]
    fn check_change_refuses_each_rule() {
        let map = Keymap::defaults();
        assert!(matches!(
            keymap::check_change(&map, Action::HideApp, Some("cmd-k")),
            Err(KeymapError::Locked(_))
        ));
        assert!(matches!(
            keymap::check_change(&map, Action::Quit, None),
            Err(KeymapError::CannotUnbind(_))
        ));
        assert!(matches!(
            keymap::check_change(&map, Action::CopyCode, None),
            Err(KeymapError::NotBindable(_))
        ));
        assert!(matches!(
            keymap::check_change(&map, Action::Zoom, Some("cmd-q")),
            Err(KeymapError::Reserved {
                owner: Some(Action::Quit),
                ..
            })
        ));
        assert!(matches!(
            keymap::check_change(&map, Action::Zoom, Some("cmd-c")),
            Err(KeymapError::Reserved { owner: None, .. })
        ));
        let (map, _) = keymap::resolve(&[("zoom".into(), "cmd-k".into())]);
        let err = keymap::check_change(&map, Action::ReviewPlugins, Some("cmd-k")).unwrap_err();
        assert!(matches!(
            err,
            KeymapError::InUse {
                action: Action::Zoom,
                ..
            }
        ));
        assert_eq!(err.to_string(), "Cmd+K is already used by Zoom");
    }
    #[test]
    fn check_change_allows_moving_quit_but_cmd_q_stays_reserved() {
        assert!(
            keymap::check_change(&Keymap::defaults(), Action::Quit, Some("cmd-shift-q")).is_ok()
        );
        let (_, issues) = keymap::resolve(&[
            ("quit".into(), "cmd-shift-q".into()),
            ("zoom".into(), "cmd-q".into()),
        ]);
        assert_eq!(issues.len(), 1);
    }
    #[test]
    fn an_optional_action_can_be_unbound_and_a_required_one_can_be_replaced() {
        assert_eq!(keymap::rule(Action::Zoom), Some(KeyRule::Optional));
        assert_eq!(
            keymap::check_change(&Keymap::defaults(), Action::Zoom, None).unwrap(),
            None
        );
        assert!(
            keymap::check_change(&Keymap::defaults(), Action::Quit, Some("cmd-shift-q")).is_ok()
        );
    }

    #[test]
    fn restored_default_displaces_a_custom_chord() {
        let (map, issues) = keymap::resolve(&[
            ("review_plugins".into(), "alt-cmd-h".into()),
            ("hide_others".into(), "cmd-k".into()),
            ("open_settings".into(), "cmd-k".into()),
        ]);
        assert_eq!(map.chord(Action::HideOthers), Some(&chord("alt-cmd-h")));
        assert_eq!(map.chord(Action::ReviewPlugins), None);
        assert_eq!(issues.len(), 2);
    }
    #[test]
    fn rows_skip_copy_actions_and_flag_custom_rows() {
        let (map, _) = keymap::resolve(&[("zoom".into(), "cmd-k".into())]);
        let rows = keymap::rows(&map);
        assert!(!rows.iter().any(|r| r.action.needs_target()));
        assert!(
            rows.iter()
                .find(|r| r.action == Action::Zoom)
                .unwrap()
                .custom
        );
    }
    #[test]
    fn rows_show_a_removed_optional_chord_as_none() {
        let (map, _) = keymap::resolve(&[("hide_others".into(), "".into())]);
        let row = keymap::rows(&map)
            .into_iter()
            .find(|r| r.action == Action::HideOthers)
            .unwrap();
        assert_eq!(row.chord, None);
        assert!(row.default.is_some());
    }
}
