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
    structure,
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

/// One session file's writes: when it was read, and each file and text.
type SessionWrites = (SystemTime, Vec<(PathBuf, String)>);

/// Reads what Pi wrote from its session store, reusing unchanged session files.
#[derive(Default)]
pub struct Records {
    sessions: Mutex<HashMap<PathBuf, SessionWrites>>,
}

impl Records {
    /// Blocking: reads session files. Paths are canonical, like workspace paths.
    pub fn read(&self, root: &Path) -> PiLines {
        let mut cache = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let listed = sessions::list(root);
        cache.retain(|path, _| listed.iter().any(|s| s.path == *path));
        let mut lines: HashMap<PathBuf, HashSet<String>> = HashMap::new();
        for summary in listed {
            let fresh = cache
                .get(&summary.path)
                .is_some_and(|(modified, _)| *modified == summary.modified);
            if !fresh {
                let (cwd, written) = sessions::written(&summary.path);
                let cwd = cwd.unwrap_or_else(|| root.to_path_buf());
                let written = written
                    .into_iter()
                    .map(|(target, text)| {
                        let path = cwd.join(target);
                        (path.canonicalize().unwrap_or(path), text)
                    })
                    .collect();
                cache.insert(summary.path.clone(), (summary.modified, written));
            }
            for (path, text) in &cache[&summary.path].1 {
                lines
                    .entry(path.clone())
                    .or_default()
                    .extend(text.lines().filter_map(significant).map(str::to_owned));
            }
        }
        Arc::new(
            lines
                .into_iter()
                .map(|(path, lines)| (path, Arc::new(lines)))
                .collect(),
        )
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
        let me = me?;
        let blame = repo.blame(path).ok()??;
        let others: Vec<bool> = blame.authors.iter().map(|author| author != me).collect();
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
        if context.pi.as_ref().is_some_and(|pi| pi.contains(content)) {
            return Some(Origin::Pi);
        }
        let others = committed.as_ref().is_some_and(|(authorship, diff)| {
            diff.unchanged_base_line(line)
                .and_then(|base| authorship.others.get(base).copied())
                .unwrap_or(false)
        });
        Some(if others { Origin::Others } else { Origin::You })
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

/// Whether each hunk only reformats: the same text once whitespace is
/// ignored, or, with difftastic installed, no change to the syntax tree.
pub fn formatting(
    name: &str,
    base: Option<&str>,
    text: &str,
    hunks: &[Hunk],
    base_lines: &[&str],
    current_lines: &[&str],
) -> Vec<bool> {
    let Some(base) = base else {
        return vec![false; hunks.len()];
    };
    const STRUCTURAL_LIMIT: usize = 512 * 1024;
    let structural = (base.len() + text.len() < STRUCTURAL_LIMIT && !hunks.is_empty())
        .then(|| structure::compare(name, base, text))
        .flatten();
    let squash = |lines: &[&str]| -> String {
        lines
            .iter()
            .flat_map(|line| line.chars())
            .filter(|c| !c.is_whitespace())
            .collect()
    };
    hunks
        .iter()
        .map(|hunk| {
            let removed = base_lines.get(hunk.base.clone()).unwrap_or_default();
            let added = current_lines.get(hunk.current.clone()).unwrap_or_default();
            squash(removed) == squash(added)
                || structural.as_ref().is_some_and(|changed| {
                    !hunk.base.clone().any(|l| changed.base.contains(&l))
                        && !hunk.current.clone().any(|l| changed.current.contains(&l))
                })
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
    fn whitespace_only_hunks_are_formatting() {
        let base = "fn a() {\n  x(1,2);\n}\nold();\n";
        let text = "fn a() {\n    x(1, 2);\n}\nnew();\n";
        let diff = LineDiff::compute(base, text);
        let hunks: Vec<Hunk> = diff.hunks_iter().collect();
        let marks = formatting(
            "a.unknown",
            Some(base),
            text,
            &hunks,
            &lines(base),
            &lines(text),
        );
        assert_eq!(marks, [true, false]);
        assert_eq!(
            formatting("a.unknown", None, text, &hunks, &[], &lines(text)),
            [false, false]
        );
    }
}
