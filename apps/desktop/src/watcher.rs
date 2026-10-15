//! Watches the workspace so changes made outside the editor (a terminal,
//! a formatter, `git checkout`) fold into open files like Pi's do.
use floem::{
    action::exec_after,
    ext_event::{ExtSendTrigger, register_ext_trigger},
    reactive::{Effect, Scope},
};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::{
    cell::Cell,
    path::{Component, Path},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// A burst of writes (a build, a checkout) becomes one refresh.
const SETTLE: Duration = Duration::from_millis(300);

/// Paths whose changes never matter to open files or the lens.
fn ignored(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(component, Component::Normal(name)
            if name == ".git" || name == "target" || name == "node_modules")
    })
}

fn relevant(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) && event.paths.iter().any(|path| !ignored(path))
}

/// Calls `changed` on the UI thread after the workspace settles. Dropping
/// the returned watcher stops it.
pub fn watch(
    scope: Scope,
    root: &Path,
    changed: impl Fn() + 'static,
) -> Option<notify::RecommendedWatcher> {
    let dirty = Arc::new(AtomicBool::new(false));
    let trigger = scope.enter(ExtSendTrigger::new);
    let flagged = dirty.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        if result.is_ok_and(|event| relevant(&event)) && !flagged.swap(true, Ordering::AcqRel) {
            register_ext_trigger(trigger);
        }
    })
    .ok()?;
    watcher.watch(root, RecursiveMode::Recursive).ok()?;
    let changed = Rc::new(changed);
    let generation = Rc::new(Cell::new(0u64));
    scope.enter(|| {
        Effect::new(move |_| {
            trigger.track();
            if !dirty.swap(false, Ordering::AcqRel) {
                return;
            }
            let current = generation.get() + 1;
            generation.set(current);
            let (generation, changed) = (generation.clone(), changed.clone());
            exec_after(SETTLE, move |_| {
                if generation.get() == current {
                    changed();
                }
            });
        })
    });
    Some(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_vcs_folders_are_ignored() {
        assert!(ignored(Path::new("/p/.git/objects/ab")));
        assert!(ignored(Path::new("/p/target/debug/x")));
        assert!(!ignored(Path::new("/p/src/main.rs")));
        assert!(!ignored(Path::new("/p/.github/workflows/ci.yml")));
    }
}
