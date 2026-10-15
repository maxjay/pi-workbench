//! The pane tree: panes side by side or stacked, split as far as wanted, each
//! pane a row of tabs showing files, Pi sessions or terminals.
use floem::{
    prelude::*,
    reactive::{Effect, ReadSignal, Scope},
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    rc::Rc,
};
use workbench_core::layout::{SavedNode, SavedTab};

pub type PaneId = u64;

/// What a tab shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    File(PathBuf),
    /// A Pi session, by its key.
    Session(u64),
    /// A terminal, by its key.
    Terminal(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    fn horizontal(self) -> bool {
        matches!(self, Direction::Left | Direction::Right)
    }
    fn after(self) -> bool {
        matches!(self, Direction::Right | Direction::Down)
    }
}

/// One pane: its tabs and which one shows, and where its cursor is.
pub struct Pane {
    pub id: PaneId,
    pub tabs: RwSignal<Vec<Tab>>,
    pub active: RwSignal<Option<Tab>>,
    /// The file the pane is waiting to show once it has been read.
    pub requested: RwSignal<Option<PathBuf>>,
    pub cursor_line: RwSignal<usize>,
    pub cursor_col: RwSignal<usize>,
    /// Where each file was left in this pane: line, character and scroll.
    /// Kept here, as the pane's view is rebuilt when the layout changes.
    pub positions: RefCell<HashMap<PathBuf, (usize, usize, f64)>>,
}

impl Pane {
    pub fn active_file(&self) -> Option<PathBuf> {
        match self.active.get_untracked() {
            Some(Tab::File(path)) => Some(path),
            _ => None,
        }
    }
    pub fn has(&self, tab: &Tab) -> bool {
        self.tabs.with_untracked(|tabs| tabs.contains(tab))
    }
}

/// The layout: a pane, or panes split side by side or stacked. Sizes are
/// signals of their own, so resizing does not rebuild the tree.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    Split {
        horizontal: bool,
        children: Vec<Node>,
        sizes: RwSignal<Vec<f64>>,
    },
}

/// A pane's place in the window, as fractions of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Node {
    pub fn leaves(&self) -> Vec<PaneId> {
        match self {
            Node::Leaf(id) => vec![*id],
            Node::Split { children, .. } => children.iter().flat_map(Node::leaves).collect(),
        }
    }

    fn rects(&self, at: Rect, out: &mut BTreeMap<PaneId, Rect>) {
        match self {
            Node::Leaf(id) => {
                out.insert(*id, at);
            }
            Node::Split {
                horizontal,
                children,
                sizes,
            } => {
                let sizes = sizes.get_untracked();
                let mut offset = 0.0;
                for (index, child) in children.iter().enumerate() {
                    let share = sizes
                        .get(index)
                        .copied()
                        .unwrap_or(1.0 / children.len() as f64);
                    let part = if *horizontal {
                        Rect {
                            x: at.x + offset * at.w,
                            w: share * at.w,
                            ..at
                        }
                    } else {
                        Rect {
                            y: at.y + offset * at.h,
                            h: share * at.h,
                            ..at
                        }
                    };
                    child.rects(part, out);
                    offset += share;
                }
            }
        }
    }
}

pub struct Panes {
    scope: Scope,
    tree: RwSignal<Node>,
    panes: RefCell<BTreeMap<PaneId, Rc<Pane>>>,
    next: Cell<PaneId>,
    focused: RwSignal<PaneId>,
    maximized: RwSignal<Option<PaneId>>,
}

impl Panes {
    pub fn new(scope: Scope) -> Rc<Self> {
        let panes = Rc::new(Self {
            scope,
            tree: scope.enter(|| RwSignal::new(Node::Leaf(1))),
            panes: RefCell::new(BTreeMap::new()),
            next: Cell::new(1),
            focused: scope.enter(|| RwSignal::new(1)),
            maximized: scope.enter(|| RwSignal::new(None)),
        });
        panes.add(1);
        panes
    }

    fn add(&self, id: PaneId) -> Rc<Pane> {
        let pane = self.scope.enter(|| {
            Rc::new(Pane {
                id,
                tabs: RwSignal::new(Vec::new()),
                active: RwSignal::new(None),
                requested: RwSignal::new(None),
                cursor_line: RwSignal::new(0),
                cursor_col: RwSignal::new(0),
                positions: RefCell::new(HashMap::new()),
            })
        });
        self.panes.borrow_mut().insert(id, pane.clone());
        pane
    }

    fn fresh(&self) -> Rc<Pane> {
        let id = self.next.get() + 1;
        self.next.set(id);
        self.add(id)
    }

    fn sizes(&self, sizes: Vec<f64>) -> RwSignal<Vec<f64>> {
        self.scope.enter(|| RwSignal::new(sizes))
    }

    /// Sets the tree and, if given, the focus, in one update, and leaves a
    /// maximised pane only when the layout really changes: Floem must not
    /// build and drop a pane's views in the same frame.
    fn set_layout(&self, tree: Node, focus: Option<PaneId>) {
        Effect::batch(|| {
            if self.maximized.with_untracked(Option::is_some) {
                self.maximized.set(None);
            }
            if let Some(focus) = focus
                && self.focused.get_untracked() != focus
            {
                self.focused.set(focus);
            }
            if self.tree.with_untracked(|now| *now != tree) {
                self.tree.set(tree);
            }
        });
    }

    pub fn tree(&self) -> ReadSignal<Node> {
        self.tree.read_only()
    }
    pub fn focused(&self) -> ReadSignal<PaneId> {
        self.focused.read_only()
    }
    pub fn maximized(&self) -> ReadSignal<Option<PaneId>> {
        self.maximized.read_only()
    }

    pub fn pane(&self, id: PaneId) -> Option<Rc<Pane>> {
        self.panes.borrow().get(&id).cloned()
    }

    pub fn focused_pane(&self) -> Rc<Pane> {
        let id = self.focused.get_untracked();
        self.pane(id)
            .or_else(|| self.panes.borrow().values().next().cloned())
            .expect("there is always a pane")
    }

    pub fn focus(&self, id: PaneId) {
        if self.panes.borrow().contains_key(&id) && self.focused.get_untracked() != id {
            self.focused.set(id);
        }
    }

    /// Panes in reading order.
    pub fn all(&self) -> Vec<Rc<Pane>> {
        self.tree
            .with_untracked(Node::leaves)
            .into_iter()
            .filter_map(|id| self.pane(id))
            .collect()
    }

    pub fn toggle_maximized(&self, id: PaneId) {
        let next = if self.maximized.get_untracked() == Some(id) {
            None
        } else {
            Some(id)
        };
        self.maximized.set(next);
    }

    /// Splits a pane, putting a new empty pane on the given side of it.
    pub fn split(&self, id: PaneId, direction: Direction) -> Rc<Pane> {
        let new = self.fresh();
        let tree = self.tree.get_untracked();
        let split = self.split_node(&tree, id, new.id, direction);
        self.set_layout(split, None);
        new
    }

    /// Adds a pane along one whole side of the window, taking `share` of it.
    pub fn split_root(&self, direction: Direction, share: f64) -> Rc<Pane> {
        let new = self.fresh();
        let tree = self.tree.get_untracked();
        let (children, sizes) = if direction.after() {
            (vec![tree, Node::Leaf(new.id)], vec![1.0 - share, share])
        } else {
            (vec![Node::Leaf(new.id), tree], vec![share, 1.0 - share])
        };
        let sizes = self.sizes(sizes);
        self.set_layout(
            Node::Split {
                horizontal: direction.horizontal(),
                children,
                sizes,
            },
            None,
        );
        new
    }

    /// Puts every tab into one pane, the focused pane's tab showing.
    pub fn collapse(&self) {
        let showing = self.focused_pane().active.get_untracked();
        let mut tabs: Vec<Tab> = Vec::new();
        for pane in self.all() {
            for tab in pane.tabs.get_untracked() {
                if !tabs.contains(&tab) {
                    tabs.push(tab);
                }
            }
        }
        let keep = self.focused_pane();
        self.panes.borrow_mut().retain(|id, _| *id == keep.id);
        keep.tabs.set(tabs);
        keep.active.set(showing);
        self.set_layout(Node::Leaf(keep.id), Some(keep.id));
    }

    fn split_node(&self, node: &Node, target: PaneId, new: PaneId, direction: Direction) -> Node {
        match node {
            Node::Leaf(id) if *id == target => {
                let children = if direction.after() {
                    vec![Node::Leaf(*id), Node::Leaf(new)]
                } else {
                    vec![Node::Leaf(new), Node::Leaf(*id)]
                };
                Node::Split {
                    horizontal: direction.horizontal(),
                    children,
                    sizes: self.sizes(vec![0.5, 0.5]),
                }
            }
            Node::Leaf(_) => node.clone(),
            Node::Split {
                horizontal,
                children,
                sizes,
            } => {
                let direct = children.iter().position(|c| *c == Node::Leaf(target));
                match direct {
                    // Splitting along the split's own direction adds a sibling,
                    // sharing the pane's space with it.
                    Some(index) if *horizontal == direction.horizontal() => {
                        let mut children = children.clone();
                        let at = if direction.after() { index + 1 } else { index };
                        children.insert(at, Node::Leaf(new));
                        let mut shares = sizes.get_untracked();
                        let half = shares.get(index).copied().unwrap_or(0.5) / 2.0;
                        if let Some(share) = shares.get_mut(index) {
                            *share = half;
                        }
                        shares.insert(at, half);
                        sizes.set(shares);
                        Node::Split {
                            horizontal: *horizontal,
                            children,
                            sizes: *sizes,
                        }
                    }
                    _ => Node::Split {
                        horizontal: *horizontal,
                        children: children
                            .iter()
                            .map(|child| self.split_node(child, target, new, direction))
                            .collect(),
                        sizes: *sizes,
                    },
                }
            }
        }
    }

    /// Removes a pane, giving its space to its neighbours. The last pane stays.
    pub fn close(&self, id: PaneId) -> bool {
        if self.panes.borrow().len() <= 1 || !self.panes.borrow().contains_key(&id) {
            return false;
        }
        let tree = self.tree.get_untracked();
        let order = tree.leaves();
        let Some(remaining) = Self::remove_node(&tree, id) else {
            return false;
        };
        self.panes.borrow_mut().remove(&id);
        // Focus moves to the pane before it, as tabs do.
        let focus = (self.focused.get_untracked() == id).then(|| {
            let at = order.iter().position(|&p| p == id).unwrap_or(0);
            order
                .iter()
                .take(at)
                .next_back()
                .or_else(|| order.get(at + 1))
                .copied()
                .unwrap_or(1)
        });
        self.set_layout(remaining, focus);
        true
    }

    fn remove_node(node: &Node, target: PaneId) -> Option<Node> {
        match node {
            Node::Leaf(id) if *id == target => None,
            Node::Leaf(_) => Some(node.clone()),
            Node::Split {
                horizontal,
                children,
                sizes,
            } => {
                let shares = sizes.get_untracked();
                let kept: Vec<(Node, f64)> = children
                    .iter()
                    .enumerate()
                    .filter_map(|(index, child)| {
                        let share = shares.get(index).copied().unwrap_or(0.5);
                        Some((Self::remove_node(child, target)?, share))
                    })
                    .collect();
                match kept.len() {
                    0 => None,
                    1 => kept.into_iter().next().map(|(child, _)| child),
                    _ => {
                        let total: f64 = kept.iter().map(|(_, share)| share).sum();
                        let (children, shares): (Vec<Node>, Vec<f64>) = kept
                            .into_iter()
                            .map(|(child, share)| (child, share / total))
                            .unzip();
                        if shares != sizes.get_untracked() {
                            sizes.set(shares);
                        }
                        Some(Node::Split {
                            horizontal: *horizontal,
                            children,
                            sizes: *sizes,
                        })
                    }
                }
            }
        }
    }

    pub fn rects(&self) -> BTreeMap<PaneId, Rect> {
        let mut rects = BTreeMap::new();
        self.tree.with_untracked(|tree| {
            tree.rects(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                },
                &mut rects,
            )
        });
        rects
    }

    /// The pane next to `id` on the given side: the one that overlaps it
    /// most along that side, nearest first.
    pub fn neighbour(&self, id: PaneId, direction: Direction) -> Option<PaneId> {
        const EPS: f64 = 1e-6;
        let rects = self.rects();
        let from = rects.get(&id)?;
        rects
            .iter()
            .filter(|(other, _)| **other != id)
            .filter_map(|(other, r)| {
                let (gap, overlap) = match direction {
                    Direction::Right => {
                        (r.x - (from.x + from.w), overlap(r.y, r.h, from.y, from.h))
                    }
                    Direction::Left => (from.x - (r.x + r.w), overlap(r.y, r.h, from.y, from.h)),
                    Direction::Down => (r.y - (from.y + from.h), overlap(r.x, r.w, from.x, from.w)),
                    Direction::Up => (from.y - (r.y + r.h), overlap(r.x, r.w, from.x, from.w)),
                };
                (gap > -EPS && overlap > EPS).then_some((*other, gap, overlap))
            })
            .min_by(|a, b| {
                a.1.partial_cmp(&b.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
            })
            .map(|(other, _, _)| other)
    }

    /// Shows a tab in a pane, adding it after the active one if it is new.
    pub fn show(&self, id: PaneId, tab: Tab) {
        let Some(pane) = self.pane(id) else { return };
        if !pane.has(&tab) {
            let after = pane.active.get_untracked();
            pane.tabs.update(|tabs| {
                let at = after
                    .and_then(|active| tabs.iter().position(|t| *t == active))
                    .map_or(tabs.len(), |i| i + 1);
                tabs.insert(at, tab.clone());
            });
        }
        if pane.active.get_untracked().as_ref() != Some(&tab) {
            pane.active.set(Some(tab));
        }
    }

    /// Takes a tab out of a pane; the tab before it shows instead.
    pub fn remove(&self, id: PaneId, tab: &Tab) {
        let Some(pane) = self.pane(id) else { return };
        let Some(index) = pane
            .tabs
            .with_untracked(|tabs| tabs.iter().position(|t| t == tab))
        else {
            return;
        };
        pane.tabs.update(|tabs| {
            tabs.remove(index);
        });
        if pane.active.get_untracked().as_ref() == Some(tab) {
            let next = pane
                .tabs
                .with_untracked(|tabs| tabs.get(index.saturating_sub(1)).or(tabs.first()).cloned());
            pane.active.set(next);
        }
    }

    pub fn remove_everywhere(&self, tab: &Tab) {
        for pane in self.all() {
            self.remove(pane.id, tab);
        }
    }

    /// The panes showing a tab among their tabs.
    pub fn holding(&self, tab: &Tab) -> Vec<PaneId> {
        self.all()
            .into_iter()
            .filter(|pane| pane.has(tab))
            .map(|pane| pane.id)
            .collect()
    }

    /// Where a tab already is: the focused pane if it has it, else the first.
    pub fn find(&self, tab: &Tab) -> Option<PaneId> {
        let holding = self.holding(tab);
        let focused = self.focused.get_untracked();
        holding
            .iter()
            .copied()
            .find(|&id| id == focused)
            .or_else(|| holding.first().copied())
    }

    /// Moves a tab from one pane to another, before the tab at `index`.
    pub fn move_tab(&self, from: PaneId, tab: &Tab, to: PaneId, index: Option<usize>) {
        if from == to {
            if let (Some(pane), Some(index)) = (self.pane(to), index) {
                pane.tabs.update(|tabs| {
                    if let Some(at) = tabs.iter().position(|t| t == tab) {
                        let tab = tabs.remove(at);
                        let index = if index > at { index - 1 } else { index };
                        tabs.insert(index.min(tabs.len()), tab);
                    }
                });
            }
            return;
        }
        let Some(target) = self.pane(to) else { return };
        self.remove(from, tab);
        if !target.has(tab) {
            target.tabs.update(|tabs| {
                let at = index.unwrap_or(tabs.len()).min(tabs.len());
                tabs.insert(at, tab.clone());
            });
        }
        target.active.set(Some(tab.clone()));
        self.focus(to);
    }

    /// The tree for saving, with each tab as `save` describes it.
    pub fn save(&self, save: impl Fn(&Tab) -> Option<SavedTab>) -> SavedNode {
        self.tree.with_untracked(|tree| self.save_node(tree, &save))
    }

    fn save_node(&self, node: &Node, save: &impl Fn(&Tab) -> Option<SavedTab>) -> SavedNode {
        match node {
            Node::Leaf(id) => {
                let pane = self.pane(*id);
                let (tabs, active) = pane.map_or((Vec::new(), None), |pane| {
                    let all = pane.tabs.get_untracked();
                    let active = pane.active.get_untracked();
                    let mut saved = Vec::new();
                    let mut index = None;
                    for tab in &all {
                        if let Some(entry) = save(tab) {
                            if Some(tab) == active.as_ref() {
                                index = Some(saved.len());
                            }
                            saved.push(entry);
                        }
                    }
                    (saved, index)
                });
                SavedNode::Pane { tabs, active }
            }
            Node::Split {
                horizontal,
                children,
                sizes,
            } => SavedNode::Split {
                horizontal: *horizontal,
                sizes: sizes.get_untracked(),
                children: children.iter().map(|c| self.save_node(c, save)).collect(),
            },
        }
    }

    /// Replaces every pane with the saved tree, each tab made by `restore`.
    /// Returns the panes in reading order.
    pub fn restore(
        &self,
        saved: &SavedNode,
        mut restore: impl FnMut(&SavedTab) -> Option<Tab>,
    ) -> Vec<PaneId> {
        self.panes.borrow_mut().clear();
        self.next.set(0);
        let tree = self.restore_node(saved, &mut restore);
        let order = tree.leaves();
        self.set_layout(tree, Some(order.first().copied().unwrap_or(1)));
        order
    }

    fn restore_node(
        &self,
        saved: &SavedNode,
        restore: &mut impl FnMut(&SavedTab) -> Option<Tab>,
    ) -> Node {
        match saved {
            SavedNode::Pane { tabs, active } => {
                let pane = self.fresh();
                let mut made = Vec::new();
                let mut shown = None;
                for (index, saved) in tabs.iter().enumerate() {
                    if let Some(tab) = restore(saved).filter(|tab| !made.contains(tab)) {
                        if Some(index) == *active {
                            shown = Some(tab.clone());
                        }
                        made.push(tab);
                    }
                }
                let shown = shown.or_else(|| made.first().cloned());
                pane.tabs.set(made);
                pane.active.set(shown);
                Node::Leaf(pane.id)
            }
            SavedNode::Split {
                horizontal,
                sizes,
                children,
            } => Node::Split {
                horizontal: *horizontal,
                children: children
                    .iter()
                    .map(|c| self.restore_node(c, restore))
                    .collect(),
                sizes: self.sizes(sizes.clone()),
            },
        }
    }
}

fn overlap(a: f64, a_len: f64, b: f64, b_len: f64) -> f64 {
    ((a + a_len).min(b + b_len) - a.max(b)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_panes(test: impl FnOnce(Rc<Panes>)) {
        let scope = Scope::new();
        scope.enter(|| test(Panes::new(scope)));
        scope.dispose();
    }

    #[test]
    fn splitting_along_a_split_adds_a_sibling_and_shares_its_space() {
        with_panes(|panes| {
            let first = panes.focused_pane().id;
            let right = panes.split(first, Direction::Right).id;
            let middle = panes.split(first, Direction::Right).id;
            assert_eq!(
                panes.tree().get_untracked().leaves(),
                [first, middle, right]
            );
            let rects = panes.rects();
            assert_eq!(rects[&first].w, 0.25);
            assert_eq!(rects[&middle].w, 0.25);
            assert_eq!(rects[&right].w, 0.5);
            // Across the split's direction, the pane becomes a split of its own.
            let below = panes.split(right, Direction::Down).id;
            assert_eq!(panes.rects()[&below].y, 0.5);
            assert_eq!(panes.neighbour(first, Direction::Right), Some(middle));
            assert_eq!(panes.neighbour(below, Direction::Up), Some(right));
            assert_eq!(panes.neighbour(below, Direction::Left), Some(middle));
            assert_eq!(panes.neighbour(first, Direction::Left), None);
        });
    }

    #[test]
    fn closing_gives_space_back_and_collapses_splits_of_one() {
        with_panes(|panes| {
            let first = panes.focused_pane().id;
            let right = panes.split(first, Direction::Right).id;
            let below = panes.split(right, Direction::Down).id;
            panes.focus(below);
            assert!(panes.close(below));
            assert_eq!(panes.tree().get_untracked().leaves(), [first, right]);
            assert_eq!(panes.focused().get_untracked(), right);
            assert_eq!(panes.rects()[&right].h, 1.0);
            assert!(panes.close(first));
            assert_eq!(panes.tree().get_untracked(), Node::Leaf(right));
            assert!(!panes.close(right), "the last pane stays");
        });
    }

    #[test]
    fn tabs_show_move_and_fall_back() {
        with_panes(|panes| {
            let first = panes.focused_pane().id;
            let a = Tab::File("a".into());
            let b = Tab::File("b".into());
            panes.show(first, a.clone());
            panes.show(first, b.clone());
            panes.show(first, a.clone());
            let pane = panes.pane(first).unwrap();
            assert_eq!(pane.tabs.get_untracked(), [a.clone(), b.clone()]);
            assert_eq!(pane.active.get_untracked(), Some(a.clone()));
            let other = panes.split(first, Direction::Right).id;
            panes.move_tab(first, &a, other, None);
            assert_eq!(pane.tabs.get_untracked(), std::slice::from_ref(&b));
            assert_eq!(pane.active.get_untracked(), Some(b.clone()));
            assert_eq!(panes.find(&a), Some(other));
            assert_eq!(panes.focused().get_untracked(), other);
            panes.remove(first, &b);
            assert_eq!(pane.active.get_untracked(), None);
        });
    }

    #[test]
    fn the_tree_saves_and_restores() {
        with_panes(|panes| {
            let first = panes.focused_pane().id;
            panes.show(first, Tab::File("a.rs".into()));
            panes.show(first, Tab::Terminal(4));
            let below = panes.split(first, Direction::Down).id;
            panes.show(below, Tab::Session(2));
            let saved = panes.save(|tab| match tab {
                Tab::File(path) => Some(SavedTab::File(path.clone())),
                Tab::Terminal(_) => Some(SavedTab::Terminal),
                Tab::Session(_) => None,
            });
            let mut terminals = 10;
            let order = panes.restore(&saved, |tab| match tab {
                SavedTab::File(path) => Some(Tab::File(path.clone())),
                SavedTab::Terminal => {
                    terminals += 1;
                    Some(Tab::Terminal(terminals))
                }
                SavedTab::Session(_) => None,
            });
            assert_eq!(order.len(), 2);
            let top = panes.pane(order[0]).unwrap();
            assert_eq!(
                top.tabs.get_untracked(),
                [Tab::File("a.rs".into()), Tab::Terminal(11)]
            );
            assert_eq!(top.active.get_untracked(), Some(Tab::Terminal(11)));
            assert_eq!(panes.rects()[&order[1]].y, 0.5);
        });
    }
}
