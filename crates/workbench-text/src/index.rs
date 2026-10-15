//! Workspace file listing and fuzzy ranking for quick open.
use ignore::{Walk, WalkBuilder};
use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};
use std::{
    cmp::Reverse,
    path::{Path, PathBuf},
};

/// Respects ignore files and shows hidden files, but never enters generated directories.
pub(crate) fn walk(root: &Path) -> Walk {
    WalkBuilder::new(root)
        .hidden(false)
        // Ignore files apply even before `git init`.
        .require_git(false)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(|entry| {
            !(entry.file_type().is_some_and(|ty| ty.is_dir())
                && matches!(
                    entry.file_name().to_str(),
                    Some(".git" | "target" | "node_modules")
                ))
        })
        .build()
}

/// Files under `root`, relative to it and sorted. Walking stops after `limit` files.
pub fn list_files(root: &Path, limit: usize) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = walk(root)
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|ty| ty.is_file()))
        .filter_map(|entry| entry.path().strip_prefix(root).ok().map(Path::to_path_buf))
        .take(limit)
        .collect();
    files.sort();
    files
}

/// Reuses the matcher's scratch memory between keystrokes.
pub struct FileMatcher {
    matcher: Matcher,
}

impl Default for FileMatcher {
    fn default() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
        }
    }
}

impl FileMatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Best matches first, each with the sorted character positions that matched.
    /// Positions count characters of the path's display string, for highlighting.
    pub fn rank(
        &mut self,
        query: &str,
        files: &[PathBuf],
        limit: usize,
    ) -> Vec<(PathBuf, Vec<usize>)> {
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        if pattern.atoms.is_empty() {
            return files
                .iter()
                .take(limit)
                .map(|file| (file.clone(), Vec::new()))
                .collect();
        }
        let mut buffer = Vec::new();
        let mut indices = Vec::new();
        let mut matches: Vec<(u32, &PathBuf, Vec<usize>)> = files
            .iter()
            .filter_map(|file| {
                let text = file.to_string_lossy();
                indices.clear();
                let score = pattern.indices(
                    Utf32Str::new(&text, &mut buffer),
                    &mut self.matcher,
                    &mut indices,
                )?;
                indices.sort_unstable();
                indices.dedup();
                Some((score, file, indices.iter().map(|&i| i as usize).collect()))
            })
            .collect();
        // Shorter paths win ties: `main.rs` before `examples/main.rs`.
        matches.sort_by_key(|(score, file, _)| (Reverse(*score), file.as_os_str().len()));
        matches
            .into_iter()
            .take(limit)
            .map(|(_, file, indices)| (file.clone(), indices))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for directory in [
            "src/bin",
            "build",
            "target",
            "node_modules/pkg",
            ".git",
            "docs",
        ] {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        for file in [
            "src/main.rs",
            "src/lib.rs",
            "src/bin/manager.rs",
            "docs/maintenance.md",
            "build/output.rs",
            "target/debug.rs",
            "node_modules/pkg/index.js",
            ".git/config",
            ".prettierrc",
            "README.md",
        ] {
            fs::write(root.join(file), "").unwrap();
        }
        fs::write(root.join(".gitignore"), "build/\n").unwrap();
        dir
    }

    #[test]
    fn listing_respects_ignores_and_keeps_hidden_files() {
        let dir = workspace();
        let files = list_files(dir.path(), 100);
        let expected = [
            ".gitignore",
            ".prettierrc",
            "README.md",
            "docs/maintenance.md",
            "src/bin/manager.rs",
            "src/lib.rs",
            "src/main.rs",
        ]
        .map(PathBuf::from);
        assert_eq!(files, expected);
        assert_eq!(list_files(dir.path(), 2).len(), 2);
    }

    #[test]
    fn ranking_prefers_the_closest_path() {
        let dir = workspace();
        let files = list_files(dir.path(), 100);
        let mut matcher = FileMatcher::new();
        for query in ["mnrs", "main"] {
            let ranked = matcher.rank(query, &files, 10);
            assert_eq!(ranked[0].0, Path::new("src/main.rs"), "{query}");
        }
        let ranked = matcher.rank("main", &files, 10);
        assert_eq!(ranked[0].1, [4, 5, 6, 7]);
        assert!(matcher.rank("zzz", &files, 10).is_empty());
    }

    #[test]
    fn empty_query_keeps_the_listing_order() {
        let dir = workspace();
        let files = list_files(dir.path(), 100);
        let ranked = FileMatcher::new().rank("  ", &files, 3);
        let paths: Vec<_> = ranked.into_iter().map(|(path, _)| path).collect();
        assert_eq!(paths, files[..3]);
    }
}
