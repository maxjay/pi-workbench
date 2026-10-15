//! Project-wide text search over the same files quick open lists.
use crate::index::walk;
use anyhow::{Context, Result};
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use std::{
    io,
    ops::Range,
    path::{Path, PathBuf},
};

/// Matches the editor's limit; larger files cannot be opened anyway.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Keeps results readable when a match sits in minified code.
pub const MAX_LINE_CHARS: usize = 500;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    /// Otherwise `text` is matched literally.
    pub regex: bool,
    pub case_sensitive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Relative to the search root.
    pub path: PathBuf,
    /// Zero-based.
    pub line: usize,
    /// The line without its terminator, cut to `MAX_LINE_CHARS`.
    pub text: String,
    /// Byte ranges of matches within `text`.
    pub ranges: Vec<Range<usize>>,
}

/// Searches files in path order and stops after `limit` hits.
/// Binary files, oversized files and unreadable files are skipped.
pub fn search(root: &Path, query: &Query, limit: usize) -> Result<Vec<Hit>> {
    let mut hits = Vec::new();
    if query.text.is_empty() || limit == 0 {
        return Ok(hits);
    }
    let matcher = RegexMatcherBuilder::new()
        .fixed_strings(!query.regex)
        .case_insensitive(!query.case_sensitive)
        .line_terminator(Some(b'\n'))
        .build(&query.text)
        .context("Invalid regular expression")?;
    let mut searcher = SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(0))
        .line_number(true)
        .build();
    for entry in walk(root).filter_map(Result::ok) {
        if hits.len() >= limit {
            break;
        }
        let small_file = entry
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_FILE_BYTES);
        let Ok(path) = entry.path().strip_prefix(root) else {
            continue;
        };
        if !small_file {
            continue;
        }
        let mut sink = Collector {
            matcher: &matcher,
            path,
            hits: &mut hits,
            limit,
        };
        // An unreadable file should not abort the whole search.
        let _ = searcher.search_path(&matcher, entry.path(), &mut sink);
    }
    Ok(hits)
}

struct Collector<'a> {
    matcher: &'a RegexMatcher,
    path: &'a Path,
    hits: &'a mut Vec<Hit>,
    limit: usize,
}

impl Sink for Collector<'_> {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, found: &SinkMatch<'_>) -> io::Result<bool> {
        let bytes = found.bytes();
        let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
        let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
        let mut text = String::from_utf8_lossy(bytes).into_owned();
        if let Some((end, _)) = text.char_indices().nth(MAX_LINE_CHARS) {
            text.truncate(end);
        }
        let mut ranges = Vec::new();
        // Matching the decoded text keeps ranges valid even after lossy decoding.
        self.matcher
            .find_iter(text.as_bytes(), |found| {
                if !found.is_empty() {
                    ranges.push(found.start()..found.end());
                }
                true
            })
            .map_err(io::Error::other)?;
        self.hits.push(Hit {
            path: self.path.to_path_buf(),
            line: found.line_number().map_or(0, |line| line as usize - 1),
            text,
            ranges,
        });
        Ok(self.hits.len() < self.limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("ignored")).unwrap();
        fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        fs::write(
            root.join("src/main.rs"),
            "fn main() {\r\n    let total = a.b(c) + 1;\r\n    Total(); total()\r\n}\r\n",
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn total() {}\n").unwrap();
        fs::write(root.join("ignored/skip.rs"), "total\n").unwrap();
        fs::write(root.join("image.bin"), b"total\0\x01\x02").unwrap();
        dir
    }

    fn literal(text: &str) -> Query {
        Query {
            text: text.into(),
            ..Query::default()
        }
    }

    fn matched(hit: &Hit) -> Vec<&str> {
        hit.ranges
            .iter()
            .map(|range| &hit.text[range.clone()])
            .collect()
    }

    #[test]
    fn literal_search_escapes_metacharacters() {
        let dir = workspace();
        let hits = search(dir.path(), &literal("a.b(c)"), 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, Path::new("src/main.rs"));
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[0].text, "    let total = a.b(c) + 1;");
        assert_eq!(matched(&hits[0]), ["a.b(c)"]);
        assert!(
            search(dir.path(), &literal("a.b(."), 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn search_is_case_insensitive_by_default_and_skips_ignored_and_binary_files() {
        let dir = workspace();
        let hits = search(dir.path(), &literal("total"), 10).unwrap();
        let found: Vec<_> = hits
            .iter()
            .map(|hit| (hit.path.clone(), hit.line))
            .collect();
        assert_eq!(
            found,
            [
                (PathBuf::from("src/lib.rs"), 0),
                (PathBuf::from("src/main.rs"), 1),
                (PathBuf::from("src/main.rs"), 2),
            ]
        );
        assert_eq!(hits[2].ranges, [4..9, 13..18]);
        let sensitive = Query {
            case_sensitive: true,
            ..literal("Total")
        };
        assert_eq!(search(dir.path(), &sensitive, 10).unwrap().len(), 1);
    }

    #[test]
    fn regex_mode_matches_patterns_and_reports_errors() {
        let dir = workspace();
        let query = Query {
            text: r"fn \w+\(\)".into(),
            regex: true,
            case_sensitive: true,
        };
        let hits = search(dir.path(), &query, 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(matched(&hits[0]), ["fn total()"]);
        assert_eq!(matched(&hits[1]), ["fn main()"]);
        let invalid = Query {
            text: "(unclosed".into(),
            regex: true,
            ..Query::default()
        };
        let error = search(dir.path(), &invalid, 10).unwrap_err();
        assert!(format!("{error:#}").contains("Invalid regular expression"));
    }

    #[test]
    fn limit_is_honoured_and_long_lines_are_capped() {
        let dir = workspace();
        assert_eq!(search(dir.path(), &literal("total"), 2).unwrap().len(), 2);
        fs::write(dir.path().join("wide.txt"), "é".repeat(800) + "needle").unwrap();
        let hits = search(dir.path(), &literal("é"), 10).unwrap();
        assert_eq!(hits[0].text.chars().count(), MAX_LINE_CHARS);
        assert_eq!(hits[0].ranges.len(), MAX_LINE_CHARS);
    }
}
