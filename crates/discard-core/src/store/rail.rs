//! Rail layout: one ordered top-level list of ungrouped projects and groups, and an ordered member list per group.
//! A project is in at most one group. A group with no members is deleted. Groups never touch disk folders.
//! Every mutation loads the layout, changes it in memory, and writes the whole layout in one transaction.

use super::{Result, Store, StoreError};
use crate::model::ProjectId;
use rusqlite::params;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailGroup {
    pub id: String,
    /// May be empty. The UI then builds a label from the member names.
    pub name: String,
    pub color: Option<String>,
    pub collapsed: bool,
    pub project_ids: Vec<ProjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RailItem {
    Project(ProjectId),
    Group(RailGroup),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RailLayout {
    pub items: Vec<RailItem>,
}

/// Where a moved project lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RailTarget {
    TopLevel,
    Group(String),
}

impl RailLayout {
    fn contains_project(&self, id: &ProjectId) -> bool {
        self.items.iter().any(|item| match item {
            RailItem::Project(p) => p == id,
            RailItem::Group(g) => g.project_ids.contains(id),
        })
    }
    fn group_index(&self, id: &str) -> Option<usize> {
        self.items.iter().position(|item| matches!(item, RailItem::Group(g) if g.id == id))
    }
    fn group_mut(&mut self, id: &str) -> Result<&mut RailGroup> {
        self.items
            .iter_mut()
            .find_map(|item| match item {
                RailItem::Group(g) if g.id == id => Some(g),
                _ => None,
            })
            .ok_or_else(|| StoreError::NotFound(format!("rail group {id}")))
    }
    /// Remove a project from the top level or from its group. A group may stay empty until `prune`.
    fn take_project(&mut self, id: &ProjectId) {
        self.items.retain(|item| !matches!(item, RailItem::Project(p) if p == id));
        for item in &mut self.items {
            if let RailItem::Group(g) = item {
                g.project_ids.retain(|p| p != id);
            }
        }
    }
    fn prune(&mut self) {
        self.items.retain(|item| !matches!(item, RailItem::Group(g) if g.project_ids.is_empty()));
    }
}

fn new_group(project_ids: Vec<ProjectId>) -> RailGroup {
    RailGroup { id: uuid::Uuid::now_v7().to_string(), name: String::new(), color: None, collapsed: false, project_ids }
}

fn valid_color(color: &str) -> bool {
    matches!(color, "ansi-1" | "ansi-2" | "ansi-3" | "ansi-4" | "ansi-5" | "ansi-6")
        || (color.len() == 7 && color.starts_with('#') && color[1..].chars().all(|c| c.is_ascii_hexdigit()))
}

pub(super) const RAIL_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS rail_groups (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    color TEXT,
    collapsed INTEGER NOT NULL DEFAULT 0,
    position INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS rail_projects (
    project_id TEXT PRIMARY KEY REFERENCES projects(id),
    group_id TEXT REFERENCES rail_groups(id),
    position INTEGER NOT NULL);";

impl Store {
    /// The saved rail layout. Visible projects without a saved position follow in `visible_projects` order.
    pub fn rail_layout(&self) -> Result<RailLayout> {
        let visible: Vec<ProjectId> = self.visible_projects()?.into_iter().map(|p| p.id).collect();
        let saved: Vec<(String, Option<String>, i64)> = self
            .conn
            .prepare("SELECT project_id,group_id,position FROM rail_projects ORDER BY position")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let groups: Vec<(RailGroup, i64)> = self
            .conn
            .prepare("SELECT id,name,color,collapsed,position FROM rail_groups")?
            .query_map([], |r| {
                Ok((RailGroup { id: r.get(0)?, name: r.get(1)?, color: r.get(2)?, collapsed: r.get(3)?, project_ids: Vec::new() }, r.get(4)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        let is_visible = |id: &str| visible.iter().any(|p| p.0 == id);
        let mut top: Vec<(i64, RailItem)> = Vec::new();
        for (mut group, position) in groups {
            group.project_ids = saved
                .iter()
                .filter(|(id, group_id, _)| group_id.as_deref() == Some(group.id.as_str()) && is_visible(id))
                .map(|(id, _, _)| ProjectId(id.clone()))
                .collect();
            if !group.project_ids.is_empty() {
                top.push((position, RailItem::Group(group)));
            }
        }
        for (id, group_id, position) in &saved {
            if group_id.is_none() && is_visible(id) {
                top.push((*position, RailItem::Project(ProjectId(id.clone()))));
            }
        }
        top.sort_by_key(|(position, _)| *position);
        let mut layout = RailLayout { items: top.into_iter().map(|(_, item)| item).collect() };
        for id in visible {
            if !saved.iter().any(|(saved_id, _, _)| *saved_id == id.0) {
                layout.items.push(RailItem::Project(id));
            }
        }
        Ok(layout)
    }

    fn rail_write(&self, layout: &RailLayout) -> Result<()> {
        self.conn.execute("DELETE FROM rail_projects", [])?;
        self.conn.execute("DELETE FROM rail_groups", [])?;
        for (position, item) in layout.items.iter().enumerate() {
            match item {
                RailItem::Project(id) => {
                    self.conn.execute("INSERT INTO rail_projects(project_id,group_id,position) VALUES(?1,NULL,?2)", params![id.0, position as i64])?;
                }
                RailItem::Group(group) => {
                    self.conn.execute(
                        "INSERT INTO rail_groups(id,name,color,collapsed,position) VALUES(?1,?2,?3,?4,?5)",
                        params![group.id, group.name, group.color, group.collapsed, position as i64],
                    )?;
                    for (member, id) in group.project_ids.iter().enumerate() {
                        self.conn.execute("INSERT INTO rail_projects(project_id,group_id,position) VALUES(?1,?2,?3)", params![id.0, group.id, member as i64])?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Load the layout, change it, delete empty groups, and save it. One transaction. An error changes nothing.
    fn rail_mutate<T>(&self, change: impl FnOnce(&mut RailLayout) -> Result<T>) -> Result<T> {
        let tx = self.conn.unchecked_transaction()?;
        let mut layout = self.rail_layout()?;
        let out = change(&mut layout)?;
        layout.prune();
        self.rail_write(&layout)?;
        tx.commit()?;
        Ok(out)
    }

    /// Drop saved entries of hidden projects and delete groups that became empty.
    pub(super) fn rail_normalize(&self) -> Result<()> {
        self.rail_mutate(|_| Ok(()))
    }

    /// Move a project to the top level or into a group. `index` counts the target list after the project leaves it.
    /// The source group may stay in the top-level list while the index applies. It is deleted afterwards if it is empty.
    pub fn rail_move_project(&self, project: &ProjectId, target: &RailTarget, index: usize) -> Result<()> {
        self.rail_mutate(|layout| {
            if !layout.contains_project(project) {
                return Err(StoreError::NotFound(format!("rail project {}", project.0)));
            }
            if let RailTarget::Group(id) = target {
                layout.group_mut(id)?;
            }
            layout.take_project(project);
            match target {
                RailTarget::TopLevel => {
                    let at = index.min(layout.items.len());
                    layout.items.insert(at, RailItem::Project(project.clone()));
                }
                RailTarget::Group(id) => {
                    let group = layout.group_mut(id)?;
                    let at = index.min(group.project_ids.len());
                    group.project_ids.insert(at, project.clone());
                }
            }
            Ok(())
        })
    }

    /// Drop `dragged` onto the ungrouped project `target`. The new group takes the position of `target` and holds `[target, dragged]`.
    pub fn rail_group_projects(&self, dragged: &ProjectId, target: &ProjectId) -> Result<String> {
        self.rail_mutate(|layout| {
            if dragged == target {
                return Err(StoreError::Invalid("Choose two different projects.".into()));
            }
            if !layout.contains_project(dragged) {
                return Err(StoreError::NotFound(format!("rail project {}", dragged.0)));
            }
            if !layout.items.iter().any(|item| matches!(item, RailItem::Project(p) if p == target)) {
                return Err(StoreError::Invalid("Drop onto a project that is not in a group.".into()));
            }
            layout.take_project(dragged);
            let at = layout.items.iter().position(|item| matches!(item, RailItem::Project(p) if p == target)).expect("target stays at the top level");
            let group = new_group(vec![target.clone(), dragged.clone()]);
            let id = group.id.clone();
            layout.items[at] = RailItem::Group(group);
            Ok(id)
        })
    }

    /// Put one project in a new group. The group takes its place, or follows its old group at the top level.
    pub fn rail_group_project(&self, project: &ProjectId) -> Result<String> {
        self.rail_mutate(|layout| {
            if !layout.contains_project(project) {
                return Err(StoreError::NotFound(format!("rail project {}", project.0)));
            }
            let group = new_group(vec![project.clone()]);
            let id = group.id.clone();
            if let Some(at) = layout.items.iter().position(|item| matches!(item, RailItem::Project(p) if p == project)) {
                layout.items[at] = RailItem::Group(group);
            } else {
                let at = layout
                    .items
                    .iter()
                    .position(|item| matches!(item, RailItem::Group(g) if g.project_ids.contains(project)))
                    .expect("the project is in a group");
                layout.take_project(project);
                layout.items.insert(at + 1, RailItem::Group(group));
            }
            Ok(id)
        })
    }

    /// Move a group in the top-level list. `index` counts the list after the group leaves it.
    pub fn rail_move_group(&self, group: &str, index: usize) -> Result<()> {
        self.rail_mutate(|layout| {
            let at = layout.group_index(group).ok_or_else(|| StoreError::NotFound(format!("rail group {group}")))?;
            let item = layout.items.remove(at);
            layout.items.insert(index.min(layout.items.len()), item);
            Ok(())
        })
    }

    pub fn rail_rename_group(&self, group: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.chars().count() > 100 {
            return Err(StoreError::Invalid("Use a group name of at most 100 characters.".into()));
        }
        self.rail_mutate(|layout| {
            layout.group_mut(group)?.name = name.to_string();
            Ok(())
        })
    }

    /// `None` restores the default color. A color is an `ansi-1` to `ansi-6` theme slot or `#rrggbb`.
    pub fn rail_set_group_color(&self, group: &str, color: Option<&str>) -> Result<()> {
        if color.is_some_and(|c| !valid_color(c)) {
            return Err(StoreError::Invalid("Choose a valid group color.".into()));
        }
        self.rail_mutate(|layout| {
            layout.group_mut(group)?.color = color.map(str::to_string);
            Ok(())
        })
    }

    pub fn rail_set_group_collapsed(&self, group: &str, collapsed: bool) -> Result<()> {
        self.rail_mutate(|layout| {
            layout.group_mut(group)?.collapsed = collapsed;
            Ok(())
        })
    }

    /// Delete the group. Its members take its top-level position, in order.
    pub fn rail_ungroup(&self, group: &str) -> Result<()> {
        self.rail_mutate(|layout| {
            let at = layout.group_index(group).ok_or_else(|| StoreError::NotFound(format!("rail group {group}")))?;
            let RailItem::Group(removed) = layout.items.remove(at) else { unreachable!("group_index found a group") };
            let members = removed.project_ids.into_iter().map(RailItem::Project);
            layout.items.splice(at..at, members);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Project;

    fn project(store: &Store, id: &str) -> ProjectId {
        let project = Project { id: ProjectId(id.into()), name: id.into(), path: format!("/{id}") };
        store.insert_project(&project).unwrap();
        project.id
    }
    fn store_with(ids: &[&str]) -> Store {
        let store = Store::open_in_memory().unwrap();
        for id in ids {
            project(&store, id);
        }
        store
    }
    fn p(id: &str) -> ProjectId {
        ProjectId(id.into())
    }
    /// Names top-level projects and wraps each group's members in brackets, for example `a [b c] d`.
    fn shape(store: &Store) -> String {
        store
            .rail_layout()
            .unwrap()
            .items
            .iter()
            .map(|item| match item {
                RailItem::Project(id) => id.0.clone(),
                RailItem::Group(g) => format!("[{}]", g.project_ids.iter().map(|id| id.0.as_str()).collect::<Vec<_>>().join(" ")),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
    fn only_group(store: &Store) -> RailGroup {
        store.rail_layout().unwrap().items.into_iter().find_map(|item| match item { RailItem::Group(g) => Some(g), _ => None }).unwrap()
    }

    #[test]
    fn projects_without_saved_order_keep_the_visible_projects_order_and_new_ones_go_last() {
        let store = store_with(&["a", "b", "c"]);
        assert_eq!(shape(&store), "a b c");
        store.rail_move_project(&p("c"), &RailTarget::TopLevel, 0).unwrap();
        assert_eq!(shape(&store), "c a b");
        project(&store, "d");
        assert_eq!(shape(&store), "c a b d");
    }

    #[test]
    fn a_move_to_the_top_level_counts_the_list_without_the_moved_project() {
        let store = store_with(&["a", "b", "c", "d"]);
        store.rail_move_project(&p("a"), &RailTarget::TopLevel, 2).unwrap();
        assert_eq!(shape(&store), "b c a d");
        store.rail_move_project(&p("d"), &RailTarget::TopLevel, 99).unwrap();
        assert_eq!(shape(&store), "b c a d");
        store.rail_move_project(&p("b"), &RailTarget::TopLevel, 99).unwrap();
        assert_eq!(shape(&store), "c a d b");
    }

    #[test]
    fn dropping_a_project_on_another_makes_a_group_at_the_target_position() {
        let store = store_with(&["a", "b", "c", "d"]);
        let id = store.rail_group_projects(&p("d"), &p("b")).unwrap();
        assert_eq!(shape(&store), "a [b d] c");
        let group = only_group(&store);
        assert_eq!(group.id, id);
        assert_eq!((group.name.as_str(), group.color, group.collapsed), ("", None, false));
    }

    #[test]
    fn grouping_rejects_itself_a_missing_project_and_a_grouped_target_without_changing_anything() {
        let store = store_with(&["a", "b", "c"]);
        store.rail_group_projects(&p("b"), &p("a")).unwrap();
        for (dragged, target) in [("c", "c"), ("c", "a"), ("ghost", "c"), ("c", "ghost")] {
            assert!(store.rail_group_projects(&p(dragged), &p(target)).is_err(), "{dragged} onto {target}");
        }
        assert_eq!(shape(&store), "[a b] c");
    }

    #[test]
    fn dragging_a_grouped_project_onto_an_ungrouped_one_deletes_the_emptied_source_group() {
        let store = store_with(&["a", "b", "c"]);
        store.rail_group_project(&p("a")).unwrap();
        assert_eq!(shape(&store), "[a] b c");
        store.rail_group_projects(&p("a"), &p("c")).unwrap();
        assert_eq!(shape(&store), "b [c a]");
    }

    #[test]
    fn a_group_of_one_is_allowed_and_takes_the_place_of_its_project() {
        let store = store_with(&["a", "b", "c"]);
        store.rail_group_project(&p("b")).unwrap();
        assert_eq!(shape(&store), "a [b] c");
        let group = store.rail_layout().unwrap();
        assert!(matches!(&group.items[1], RailItem::Group(g) if g.project_ids == [p("b")]));
    }

    #[test]
    fn a_solo_group_of_a_grouped_project_follows_its_old_group() {
        let store = store_with(&["a", "b", "c"]);
        store.rail_group_projects(&p("b"), &p("a")).unwrap();
        store.rail_group_project(&p("b")).unwrap();
        assert_eq!(shape(&store), "[a] [b] c");
    }

    #[test]
    fn projects_move_into_a_group_at_an_index_and_back_out() {
        let store = store_with(&["a", "b", "c", "d"]);
        let group = store.rail_group_projects(&p("b"), &p("a")).unwrap();
        store.rail_move_project(&p("c"), &RailTarget::Group(group.clone()), 1).unwrap();
        assert_eq!(shape(&store), "[a c b] d");
        store.rail_move_project(&p("d"), &RailTarget::Group(group.clone()), 0).unwrap();
        assert_eq!(shape(&store), "[d a c b]");
        store.rail_move_project(&p("a"), &RailTarget::Group(group.clone()), 3).unwrap();
        assert_eq!(shape(&store), "[d c b a]");
        store.rail_move_project(&p("c"), &RailTarget::TopLevel, 0).unwrap();
        assert_eq!(shape(&store), "c [d b a]");
    }

    #[test]
    fn a_project_is_in_at_most_one_group() {
        let store = store_with(&["a", "b", "c", "d"]);
        let first = store.rail_group_projects(&p("b"), &p("a")).unwrap();
        let second = store.rail_group_projects(&p("d"), &p("c")).unwrap();
        store.rail_move_project(&p("b"), &RailTarget::Group(second), 0).unwrap();
        assert_eq!(shape(&store), "[a] [b c d]");
        let layout = store.rail_layout().unwrap();
        let members = layout.items.iter().flat_map(|item| match item { RailItem::Group(g) => g.project_ids.clone(), RailItem::Project(id) => vec![id.clone()] }).collect::<Vec<_>>();
        assert_eq!(members.len(), 4);
        assert!(matches!(&layout.items[0], RailItem::Group(g) if g.id == first));
    }

    #[test]
    fn the_last_member_leaving_deletes_the_group_and_the_top_level_index_still_applies() {
        let store = store_with(&["a", "b", "c"]);
        let group = store.rail_group_project(&p("a")).unwrap();
        // The list shown while dragging is `[a] b c`. Index 2 puts `a` before `c`.
        store.rail_move_project(&p("a"), &RailTarget::TopLevel, 2).unwrap();
        assert_eq!(shape(&store), "b a c");
        assert!(store.rail_set_group_collapsed(&group, true).is_err(), "the group is gone");
        let rows: i64 = store.conn.query_row("SELECT COUNT(*) FROM rail_groups", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn moving_the_only_member_within_its_own_group_keeps_the_group() {
        let store = store_with(&["a", "b"]);
        let group = store.rail_group_project(&p("a")).unwrap();
        store.rail_move_project(&p("a"), &RailTarget::Group(group), 0).unwrap();
        assert_eq!(shape(&store), "[a] b");
    }

    #[test]
    fn a_failed_move_changes_nothing() {
        let store = store_with(&["a", "b"]);
        assert!(store.rail_move_project(&p("a"), &RailTarget::Group("ghost".into()), 0).is_err());
        assert!(store.rail_move_project(&p("ghost"), &RailTarget::TopLevel, 0).is_err());
        assert_eq!(shape(&store), "a b");
    }

    #[test]
    fn groups_reorder_in_the_top_level_list() {
        let store = store_with(&["a", "b", "c", "d"]);
        let group = store.rail_group_projects(&p("b"), &p("a")).unwrap();
        store.rail_move_group(&group, 2).unwrap();
        assert_eq!(shape(&store), "c d [a b]");
        store.rail_move_group(&group, 1).unwrap();
        assert_eq!(shape(&store), "c [a b] d");
        store.rail_move_group(&group, 0).unwrap();
        assert_eq!(shape(&store), "[a b] c d");
        assert!(store.rail_move_group("ghost", 0).is_err());
    }

    #[test]
    fn ungroup_returns_members_to_the_group_position_in_order() {
        let store = store_with(&["a", "b", "c", "d", "e"]);
        let group = store.rail_group_projects(&p("d"), &p("b")).unwrap();
        store.rail_move_project(&p("c"), &RailTarget::Group(group.clone()), 1).unwrap();
        assert_eq!(shape(&store), "a [b c d] e");
        store.rail_ungroup(&group).unwrap();
        assert_eq!(shape(&store), "a b c d e");
        assert!(store.rail_ungroup(&group).is_err());
    }

    #[test]
    fn rename_color_and_collapse_are_saved_and_validated() {
        let store = store_with(&["a", "b"]);
        let group = store.rail_group_projects(&p("b"), &p("a")).unwrap();
        store.rail_rename_group(&group, "  Work  ").unwrap();
        store.rail_set_group_color(&group, Some("ansi-3")).unwrap();
        store.rail_set_group_collapsed(&group, true).unwrap();
        let saved = only_group(&store);
        assert_eq!((saved.name.as_str(), saved.color.as_deref(), saved.collapsed), ("Work", Some("ansi-3"), true));
        store.rail_set_group_color(&group, Some("#A1b2C3")).unwrap();
        for bad in ["ansi-7", "red", "#12345", "#gggggg", ""] {
            assert!(matches!(store.rail_set_group_color(&group, Some(bad)), Err(StoreError::Invalid(_))), "{bad}");
        }
        assert_eq!(only_group(&store).color.as_deref(), Some("#A1b2C3"));
        store.rail_set_group_color(&group, None).unwrap();
        store.rail_rename_group(&group, "   ").unwrap();
        let cleared = only_group(&store);
        assert_eq!((cleared.name.as_str(), cleared.color), ("", None));
        assert!(matches!(store.rail_rename_group(&group, &"x".repeat(101)), Err(StoreError::Invalid(_))));
        assert!(matches!(store.rail_rename_group("ghost", "x"), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn hiding_a_project_removes_it_from_its_group_and_an_emptied_group_is_deleted() {
        let store = store_with(&["a", "b", "c"]);
        let group = store.rail_group_projects(&p("b"), &p("a")).unwrap();
        store.hide_project(&p("b")).unwrap();
        assert_eq!(shape(&store), "[a] c", "a group of one stays");
        store.hide_project(&p("a")).unwrap();
        assert_eq!(shape(&store), "c");
        assert!(store.rail_set_group_collapsed(&group, true).is_err());
        let rows: i64 = store.conn.query_row("SELECT COUNT(*) FROM rail_projects", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1, "hidden projects keep no rail entry");
        store.select_project(&p("a")).unwrap();
        assert_eq!(shape(&store), "c a", "a shown project goes last, outside any group");
    }

    #[test]
    fn the_layout_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rail.db");
        {
            let store = Store::open(&path).unwrap();
            for id in ["a", "b", "c", "d"] {
                project(&store, id);
            }
            let group = store.rail_group_projects(&p("c"), &p("b")).unwrap();
            store.rail_set_group_collapsed(&group, true).unwrap();
            store.rail_rename_group(&group, "Pair").unwrap();
            store.rail_move_project(&p("d"), &RailTarget::TopLevel, 0).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(shape(&store), "d a [b c]");
        let group = only_group(&store);
        assert_eq!((group.name.as_str(), group.collapsed), ("Pair", true));
    }
}
