//! Who changed a line, and whether a change is only formatting.
//!
//! Pi's lines are recognised by content against what Pi's own session records
//! say it wrote to the file; committed lines by someone else come from git
//! blame; everything else is yours.
use pi_client::sessions;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};
use workbench_core::{
    diff::{Hunk, LineChange, LineDiff},
    git::Repo,
    structure::Changed,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    You,
    Pi,
    /// Committed by someone else.
    Others,
}

/// What Pi wrote, per file, as significant trimmed lines.
pub type PiLines = Arc<HashMap<PathBuf, Arc<HashSet<String>>>>;

/// A line worth matching on its own: blank lines and lone brackets would
/// match anywhere, so they take the origin of the lines around them.
fn significant(line: &str) -> Option<&str> {
    let line = line.trim();
    (line.chars().filter(|c| c.is_alphanumeric()).count() >= 2).then_some(line)
}

/// One session file's writes: when it changed, and each path and text.
type SessionWrites = (SystemTime, Vec<(PathBuf, String)>);

#[derive(Default)]
struct RecordsState {
    sessions: HashMap<PathBuf, SessionWrites>,
    built: Option<PiLines>,
}

/// Reads what Pi wrote from its session store, re-reading only session files
/// that changed, and handing back the same map while none did.
#[derive(Default)]
pub struct Records {
    state: Mutex<RecordsState>,
}

/// Pi's own path forms: `@file` references and `~` for the home folder.
fn resolve(cwd: &Path, target: &str) -> PathBuf {
    let target = target.strip_prefix('@').unwrap_or(target);
    let home = || std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let path = match target.strip_prefix("~/").or(target.strip_prefix("~\\")) {
        Some(rest) => {
            home().map_or_else(|| cwd.join(target), |home| PathBuf::from(home).join(rest))
        }
        None => cwd.join(target),
    };
    path.canonicalize().unwrap_or(path)
}

impl Records {
    /// Blocking: reads session files. Paths are canonical, like workspace paths.
    pub fn read(&self, root: &Path) -> PiLines {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let files = sessions::files(root);
        let before = state.sessions.len();
        state
            .sessions
            .retain(|path, _| files.iter().any(|(listed, _)| listed == path));
        let mut changed = state.sessions.len() != before;
        for (path, modified) in files {
            if state
                .sessions
                .get(&path)
                .is_some_and(|(m, _)| *m == modified)
            {
                continue;
            }
            let (cwd, written) = sessions::written(&path);
            // A shared session folder can hold other projects' sessions.
            let cwd = cwd.unwrap_or_else(|| root.to_path_buf());
            let written = if sessions::belongs(&cwd, root) {
                written
                    .into_iter()
                    .map(|(target, text)| (resolve(&cwd, &target), text))
                    .collect()
            } else {
                Vec::new()
            };
            state.sessions.insert(path, (modified, written));
            changed = true;
        }
        if !changed && let Some(built) = &state.built {
            return built.clone();
        }
        let mut lines: HashMap<PathBuf, HashSet<String>> = HashMap::new();
        for (_, written) in state.sessions.values() {
            for (path, text) in written {
                lines
                    .entry(path.clone())
                    .or_default()
                    .extend(text.lines().filter_map(significant).map(str::to_owned));
            }
        }
        let built: PiLines = Arc::new(
            lines
                .into_iter()
                .map(|(path, lines)| (path, Arc::new(lines)))
                .collect(),
        );
        state.built = Some(built.clone());
        built
    }
}

/// Which committed lines of a file someone else wrote.
pub struct Authorship {
    /// The file as committed at HEAD.
    text: String,
    others: Vec<bool>,
}

impl Authorship {
    /// Blocking: runs git blame on the committed file. `None` when there is
    /// nothing to tell apart, such as a new file or no configured email.
    pub fn read(repo: &Repo, path: &Path, me: Option<&str>) -> Option<Arc<Self>> {
        let me = me?.to_lowercase();
        let blame = repo.blame(path).ok()??;
        let others: Vec<bool> = blame
            .authors
            .iter()
            .map(|author| author.to_lowercase() != me)
            .collect();
        others.contains(&true).then(|| {
            Arc::new(Self {
                text: blame.text,
                others,
            })
        })
    }
}

/// What a comparison needs beyond the two texts.
#[derive(Clone, Default)]
pub struct Context {
    pub pi: Option<Arc<HashSet<String>>>,
    pub authorship: Option<Arc<Authorship>>,
}

/// The origin of each added line of `current`, by line; `None` elsewhere.
pub fn origins(
    diff: &LineDiff,
    hunks: &[Hunk],
    current: &[&str],
    text: &str,
    context: &Context,
) -> Vec<Option<Origin>> {
    let mut origins = vec![None; diff.current.len()];
    // Maps current lines back to the committed file to find their author.
    let committed = context
        .authorship
        .as_ref()
        .map(|a| (a, LineDiff::compute(&a.text, text)));
    let decide = |line: usize, content: &str| {
        // A line committed by someone else and untouched since stays theirs,
        // even if Pi's edits happened to repeat it.
        let others = committed.as_ref().is_some_and(|(authorship, diff)| {
            diff.unchanged_base_line(line)
                .and_then(|base| authorship.others.get(base).copied())
                .unwrap_or(false)
        });
        Some(if others {
            Origin::Others
        } else if context.pi.as_ref().is_some_and(|pi| pi.contains(content)) {
            Origin::Pi
        } else {
            Origin::You
        })
    };
    for hunk in hunks {
        let added: Vec<usize> = hunk
            .current
            .clone()
            .filter(|&line| diff.current.get(line) == Some(&LineChange::Added))
            .collect();
        for &line in &added {
            let content = current.get(line).copied().unwrap_or_default();
            origins[line] = significant(content).and_then(|content| decide(line, content));
        }
        // Lines too plain to match take the origin of the nearest line before
        // them in the hunk, or after them for leading ones.
        let mut last = None;
        for &line in &added {
            match origins[line] {
                Some(origin) => last = Some(origin),
                None => origins[line] = last,
            }
        }
        let mut next = None;
        for &line in added.iter().rev() {
            match origins[line] {
                Some(origin) => next = Some(origin),
                None => origins[line] = next,
            }
        }
        for &line in &added {
            origins[line].get_or_insert(Origin::You);
        }
    }
    origins
}

/// Files whose indentation carries meaning, where re-indenting is a change.
fn indentation_matters(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let extension = lower.rsplit_once('.').map(|(_, ext)| ext);
    matches!(lower.as_str(), "makefile" | "gnumakefile")
        || extension.is_none()
        || extension.is_some_and(|ext| {
            matches!(
                ext,
                "py" | "pyi"
                    | "pyw"
                    | "yaml"
                    | "yml"
                    | "md"
                    | "markdown"
                    | "rst"
                    | "txt"
                    | "mk"
                    | "hs"
                    | "lhs"
                    | "nim"
                    | "coffee"
                    | "haml"
                    | "pug"
                    | "jade"
                    | "slim"
                    | "sass"
                    | "styl"
                    | "fs"
                    | "fsi"
                    | "fsx"
                    | "elm"
                    | "gd"
                    | "toml"
                    | "ini"
            )
        })
}

/// Whether each hunk only reformats. Hiding a real change is far worse than
/// showing a cosmetic one, so this is conservative: without difftastic, only
/// blank lines and trailing whitespace count; with it, a hunk it reports no
/// syntax change in also counts, except where indentation carries meaning.
/// `structural` runs difftastic, and is only called when it could matter.
pub fn formatting(
    name: &str,
    base: Option<&str>,
    hunks: &[Hunk],
    base_lines: &[&str],
    current_lines: &[&str],
    structural: impl FnOnce() -> Option<Changed>,
) -> Vec<bool> {
    let Some(base) = base else {
        return vec![false; hunks.len()];
    };
    let visible = |lines: &[&str]| -> Vec<String> {
        lines
            .iter()
            .map(|line| line.trim_end().to_owned())
            .filter(|line| !line.is_empty())
            .collect()
    };
    let cosmetic: Vec<bool> = hunks
        .iter()
        .map(|hunk| {
            let removed = base_lines.get(hunk.base.clone()).unwrap_or_default();
            let added = current_lines.get(hunk.current.clone()).unwrap_or_default();
            visible(removed) == visible(added)
        })
        .collect();
    const STRUCTURAL_LIMIT: usize = 512 * 1024;
    let undecided = cosmetic.iter().any(|c| !c);
    if !undecided
        || indentation_matters(name)
        || base.len() + current_lines.iter().map(|l| l.len() + 1).sum::<usize>() > STRUCTURAL_LIMIT
    {
        return cosmetic;
    }
    let Some(changed) = structural() else {
        return cosmetic;
    };
    // Trust difftastic only if it agrees about where the changes are.
    let inside = |line: usize, side: fn(&Hunk) -> &std::ops::Range<usize>| {
        hunks.iter().any(|hunk| side(hunk).contains(&line))
    };
    let agrees = changed.base.iter().all(|&l| inside(l, |h| &h.base))
        && changed.current.iter().all(|&l| inside(l, |h| &h.current));
    if !agrees {
        return cosmetic;
    }
    hunks
        .iter()
        .zip(cosmetic)
        .map(|(hunk, cosmetic)| {
            cosmetic
                || (!hunk.base.clone().any(|l| changed.base.contains(&l))
                    && !hunk.current.clone().any(|l| changed.current.contains(&l)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<&str> {
        text.split('\n').collect()
    }

    #[test]
    fn pi_lines_are_recognised_and_brackets_follow_their_neighbours() {
        let base = "start\nend\n";
        let text = "start\n}\n    greet(name);\n}\n    mine();\nend\n";
        let diff = LineDiff::compute(base, text);
        let hunks: Vec<Hunk> = diff.hunks_iter().collect();
        assert_eq!(hunks.len(), 1);
        let pi = Arc::new(HashSet::from(["greet(name);".to_owned()]));
        // Context repeated in Pi's edits is not Pi's: see `sessions::written`.
        let context = Context {
            pi: Some(pi),
            authorship: None,
        };
        let origins = origins(&diff, &hunks, &lines(text), text, &context);
        assert_eq!(origins[0], None);
        // A leading bracket takes the next line's origin, a later one the line before.
        assert_eq!(origins[1], Some(Origin::Pi));
        assert_eq!(origins[2], Some(Origin::Pi));
        assert_eq!(origins[3], Some(Origin::Pi));
        assert_eq!(origins[4], Some(Origin::You));
        assert_eq!(origins[5], None);
    }

    #[test]
    fn committed_lines_by_others_are_theirs() {
        let base = "a\n";
        let text = "a\nalpha beta\nmine here\n";
        let authorship = Arc::new(Authorship {
            text: "a\nalpha beta\n".into(),
            others: vec![false, true],
        });
        let diff = LineDiff::compute(base, text);
        let hunks: Vec<Hunk> = diff.hunks_iter().collect();
        let context = Context {
            pi: None,
            authorship: Some(authorship),
        };
        let origins = origins(&diff, &hunks, &lines(text), text, &context);
        assert_eq!(origins[1], Some(Origin::Others));
        assert_eq!(origins[2], Some(Origin::You));
    }

    #[test]
    fn only_blank_lines_and_trailing_spaces_are_formatting_without_difftastic() {
        let base = "fn a() {\n  x(1,2);\n}\nold();\nmid();\nkeep();\nend();\n";
        let text = "fn a() {\n    x(1, 2);\n}\nnew();\nmid();\nkeep();   \n\nend();\n";
        let diff = LineDiff::compute(base, text);
        let hunks: Vec<Hunk> = diff.hunks_iter().collect();
        let marks = formatting(
            "a.rs",
            Some(base),
            &hunks,
            &lines(base),
            &lines(text),
            || None,
        );
        assert_eq!(marks, [false, false, true]);
        assert_eq!(
            formatting("a.rs", None, &hunks, &[], &lines(text), || None),
            [false, false, false]
        );
    }

    #[test]
    fn difftastic_decides_only_where_indentation_is_cosmetic() {
        let base = "if a:\n    b()\n    c()\n";
        let text = "if a:\n    b()\nc()\n";
        let diff = LineDiff::compute(base, text);
        let hunks: Vec<Hunk> = diff.hunks_iter().collect();
        let unchanged = || Some(Changed::default());
        // difftastic can call a Python dedent unchanged; it is not.
        assert_eq!(
            formatting(
                "a.py",
                Some(base),
                &hunks,
                &lines(base),
                &lines(text),
                unchanged
            ),
            [false]
        );
        assert_eq!(
            formatting(
                "a.rs",
                Some(base),
                &hunks,
                &lines(base),
                &lines(text),
                unchanged
            ),
            [true]
        );
        // A report of changes outside every hunk is not trusted.
        let elsewhere = || {
            Some(Changed {
                base: HashSet::from([0]),
                current: HashSet::new(),
            })
        };
        assert_eq!(
            formatting(
                "a.rs",
                Some(base),
                &hunks,
                &lines(base),
                &lines(text),
                elsewhere
            ),
            [false]
        );
    }
}
