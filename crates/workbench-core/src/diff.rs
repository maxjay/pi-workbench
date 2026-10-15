//! Line diffs between a baseline and the current text, independent of the renderer.
use similar::{Algorithm, DiffTag, capture_diff_slices, capture_diff_slices_deadline};
use std::{
    ops::Range,
    time::{Duration, Instant},
};

/// Large inputs fall back to a coarser diff rather than blocking.
const DIFF_DEADLINE: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineChange {
    Unchanged,
    /// Only appears on the baseline side.
    Removed,
    /// Only appears on the current side.
    Added,
}

/// Aligned runs of lines. A changed block may be empty on one side.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Block {
    base: Range<usize>,
    current: Range<usize>,
    changed: bool,
}

/// A changed region: baseline lines replaced by current lines.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hunk {
    pub base: Range<usize>,
    pub current: Range<usize>,
}

/// Per-line changes for both sides, indexed by editor line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineDiff {
    pub base: Vec<LineChange>,
    pub current: Vec<LineChange>,
    blocks: Vec<Block>,
}

/// Splits like the editor does: a trailing newline yields a final empty line.
/// Line endings are ignored so CRLF and LF checkouts compare equal.
fn lines(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

impl LineDiff {
    pub fn compute(base: &str, current: &str) -> Self {
        let (old, new) = (lines(base), lines(current));
        let deadline = Instant::now() + DIFF_DEADLINE;
        let ops = capture_diff_slices_deadline(Algorithm::Myers, &old, &new, Some(deadline));
        let mut diff = Self {
            base: vec![LineChange::Unchanged; old.len()],
            current: vec![LineChange::Unchanged; new.len()],
            blocks: Vec::new(),
        };
        for op in ops {
            let (base, current) = (op.old_range(), op.new_range());
            let changed = op.tag() != DiffTag::Equal;
            if changed {
                diff.base[base.clone()].fill(LineChange::Removed);
                diff.current[current.clone()].fill(LineChange::Added);
            }
            match diff.blocks.last_mut() {
                // A delete next to an insert is one change to the reader.
                Some(last) if changed && last.changed => {
                    last.base.end = base.end;
                    last.current.end = current.end;
                }
                _ => diff.blocks.push(Block {
                    base,
                    current,
                    changed,
                }),
            }
        }
        diff
    }

    /// Every line is new, as for a file missing from the baseline.
    pub fn all_added(current: &str) -> Self {
        let lines = lines(current);
        let mut marks = vec![LineChange::Added; lines.len()];
        // The empty line after a final newline is not content.
        if lines.last() == Some(&"")
            && let Some(last) = marks.last_mut()
        {
            *last = LineChange::Unchanged;
        }
        let changed = marks.contains(&LineChange::Added);
        Self {
            base: Vec::new(),
            current: marks,
            blocks: changed
                .then_some(Block {
                    base: 0..0,
                    current: 0..lines.len(),
                    changed: true,
                })
                .into_iter()
                .collect(),
        }
    }

    /// Each changed region, in order. Either range may be empty.
    pub fn hunks_iter(&self) -> impl Iterator<Item = Hunk> + '_ {
        self.blocks
            .iter()
            .filter(|block| block.changed)
            .map(|block| Hunk {
                base: block.base.clone(),
                current: block.current.clone(),
            })
    }

    /// Number of separate changed regions.
    pub fn hunks(&self) -> usize {
        self.blocks.iter().filter(|block| block.changed).count()
    }

    pub fn added(&self) -> usize {
        self.current
            .iter()
            .filter(|&&c| c == LineChange::Added)
            .count()
    }

    pub fn removed(&self) -> usize {
        self.base
            .iter()
            .filter(|&&c| c == LineChange::Removed)
            .count()
    }

    /// `base` with only the changes that touch `lines` of `current` taken from
    /// `current`. Untouched lines of `base` are kept byte for byte; lines taken
    /// from `current` get `base`'s usual line ending. Used to stage part of a file.
    pub fn apply_lines(base: &str, current: &str, lines: Range<usize>) -> String {
        let (old, new) = (pieces(base), pieces(current));
        let crlf = old.iter().filter(|(_, end)| *end == "\r\n").count();
        let lf = old.iter().filter(|(_, end)| *end == "\n").count();
        let ending = if crlf > lf { "\r\n" } else { "\n" };
        let (old_text, new_text): (Vec<&str>, Vec<&str>) = (
            old.iter().map(|(line, _)| *line).collect(),
            new.iter().map(|(line, _)| *line).collect(),
        );
        // Operation by operation, not hunk by hunk: neighbouring changes that a
        // hunk would merge stay apart, so only what `lines` covers is taken.
        let ops = capture_diff_slices(Algorithm::Myers, &old_text, &new_text);
        let touches = |range: Range<usize>| {
            if range.is_empty() {
                // A removal sits between lines; it belongs to a selection
                // that starts there or covers it.
                if lines.is_empty() {
                    range.start == lines.start
                } else {
                    lines.start <= range.start && range.start < lines.end
                }
            } else if lines.is_empty() {
                false
            } else {
                range.start < lines.end && lines.start < range.end
            }
        };
        let mut out = String::with_capacity(base.len());
        let keep = |out: &mut String, range: Range<usize>| {
            for (line, end) in &old[range] {
                out.push_str(line);
                out.push_str(end);
            }
        };
        let take = |out: &mut String, range: Range<usize>| {
            for (line, end) in &new[range] {
                out.push_str(line);
                // The end of the file keeps having, or not having, a newline.
                out.push_str(if end.is_empty() { "" } else { ending });
            }
        };
        for op in ops {
            let (old_range, new_range) = (op.old_range(), op.new_range());
            match op.tag() {
                DiffTag::Equal => keep(&mut out, old_range),
                // Line-for-line replacements are decided line by line, since
                // neighbouring edits arrive as one replacement.
                DiffTag::Replace if old_range.len() == new_range.len() => {
                    for (old_line, new_line) in old_range.zip(new_range) {
                        if touches(new_line..new_line + 1) {
                            take(&mut out, new_line..new_line + 1);
                        } else {
                            keep(&mut out, old_line..old_line + 1);
                        }
                    }
                }
                _ if touches(new_range.clone()) => take(&mut out, new_range),
                _ => keep(&mut out, old_range),
            }
        }
        out
    }

    /// The baseline line `current_line` is a copy of, if it is unchanged.
    pub fn unchanged_base_line(&self, current_line: usize) -> Option<usize> {
        let block = self
            .blocks
            .iter()
            .find(|block| !block.changed && block.current.contains(&current_line))?;
        Some(block.base.start + current_line - block.current.start)
    }

    /// The baseline line shown alongside `current_line`, for synchronised scrolling.
    pub fn base_line(&self, current_line: usize) -> usize {
        let Some(block) = self
            .blocks
            .iter()
            .find(|block| block.current.contains(&current_line))
        else {
            return self.base.len().saturating_sub(1);
        };
        let offset = current_line - block.current.start;
        let span = block.base.len().saturating_sub(1);
        let line = block.base.start
            + if block.changed {
                offset.min(span)
            } else {
                offset
            };
        // Lines inserted at the end sit past the baseline's last line.
        line.min(self.base.len().saturating_sub(1))
    }
}

/// Each line of `text` with its own ending, plus an empty last line after
/// a final newline, so indices match [`LineDiff`]'s lines.
fn pieces(text: &str) -> Vec<(&str, &str)> {
    let mut pieces: Vec<(&str, &str)> = text
        .split_inclusive('\n')
        .map(|piece| {
            let line = piece.trim_end_matches('\n');
            let line = line.strip_suffix('\r').unwrap_or(line);
            (line, &piece[line.len()..])
        })
        .collect();
    if text.is_empty() || text.ends_with('\n') {
        pieces.push(("", ""));
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;
    use LineChange::*;

    #[test]
    fn applying_lines_keeps_mixed_endings_and_separate_neighbours() {
        // One CRLF line in an LF file stays the only one.
        let base = "a\nb\r\nc\nd\n";
        assert_eq!(
            LineDiff::apply_lines(base, "a\nb\nc\nD\n", 3..4),
            "a\nb\r\nc\nD\n"
        );
        // Adjacent changes are taken apart: only line 2 is selected.
        assert_eq!(
            LineDiff::apply_lines("l1\nl2\nl3\n", "l1\nL2\nL3\n", 2..3),
            "l1\nl2\nL3\n"
        );
        // A removal elsewhere is left alone.
        assert_eq!(
            LineDiff::apply_lines("a\nb\nc\nd\ne\n", "a\nB\nc\ne\n", 1..2),
            "a\nB\nc\nd\ne\n"
        );
        // A removal is taken when selected by its position.
        assert_eq!(
            LineDiff::apply_lines("a\nb\nc\nd\ne\n", "a\nB\nc\ne\n", 3..3),
            "a\nb\nc\ne\n"
        );
        // The end of the file keeps its missing newline.
        assert_eq!(LineDiff::apply_lines("a\nb", "a\nB", 1..2), "a\nB");
        assert_eq!(LineDiff::apply_lines("a\n", "a\nb", 1..2), "a\nb");
    }

    #[test]
    fn applying_lines_takes_only_the_touched_changes() {
        let base = "a\nb\nc\nd\n";
        let current = "a\nB\nc\nD\n";
        assert_eq!(LineDiff::apply_lines(base, current, 1..2), "a\nB\nc\nd\n");
        assert_eq!(LineDiff::apply_lines(base, current, 3..4), "a\nb\nc\nD\n");
        assert_eq!(LineDiff::apply_lines(base, current, 0..4), current);
        // A removal is taken by its position.
        assert_eq!(LineDiff::apply_lines("a\nb\nc\n", "a\nc\n", 1..1), "a\nc\n");
        // The base's line endings are kept.
        assert_eq!(
            LineDiff::apply_lines("a\r\nb\r\n", "a\nB\n", 1..2),
            "a\r\nB\r\n"
        );
    }

    #[test]
    fn unchanged_lines_map_back_to_the_baseline() {
        let diff = LineDiff::compute("a\nb\nc\n", "new\na\nc\n");
        assert_eq!(diff.unchanged_base_line(0), None);
        assert_eq!(diff.unchanged_base_line(1), Some(0));
        assert_eq!(diff.unchanged_base_line(2), Some(2));
    }

    #[test]
    fn identical_text_has_no_changes_even_with_different_line_endings() {
        let diff = LineDiff::compute("a\nb\n", "a\r\nb\r\n");
        assert_eq!(diff.hunks(), 0);
        assert!(
            diff.base
                .iter()
                .chain(&diff.current)
                .all(|&c| c == Unchanged)
        );
    }

    #[test]
    fn marks_each_side_and_counts_replacements_as_one_hunk() {
        let diff = LineDiff::compute("keep\nold\nkeep\n", "keep\nnew\nnewer\nkeep\n");
        assert_eq!(diff.base, [Unchanged, Removed, Unchanged, Unchanged]);
        assert_eq!(
            diff.current,
            [Unchanged, Added, Added, Unchanged, Unchanged]
        );
        assert_eq!(diff.hunks(), 1);
        assert_eq!((diff.added(), diff.removed()), (2, 1));
    }

    #[test]
    fn new_files_count_content_lines_as_added() {
        let diff = LineDiff::all_added("one\ntwo\n");
        assert_eq!((diff.added(), diff.removed(), diff.hunks()), (2, 0, 1));
        assert_eq!(diff.base_line(1), 0);
        assert_eq!(LineDiff::all_added("").hunks(), 0);
    }

    #[test]
    fn hunks_pair_removed_and_added_lines() {
        let diff = LineDiff::compute(
            "a
b
c
d",
            "a
B
c
d
e",
        );
        let hunks: Vec<_> = diff.hunks_iter().collect();
        assert_eq!(
            hunks,
            [
                Hunk {
                    base: 1..2,
                    current: 1..2
                },
                Hunk {
                    base: 4..4,
                    current: 4..5
                }
            ]
        );
    }

    #[test]
    fn missing_baseline_marks_every_line_added() {
        let diff = LineDiff::compute("", "one\ntwo");
        assert_eq!(diff.current, [Added, Added]);
        assert_eq!(diff.base, [Removed]);
    }

    #[test]
    fn scrolling_maps_current_lines_onto_the_baseline() {
        // Two lines inserted after line 0, one line removed near the end.
        let diff = LineDiff::compute("a\nb\nc\nd\ne", "a\nx\ny\nb\nc\ne");
        assert_eq!(diff.hunks(), 2);
        assert_eq!(diff.base_line(0), 0);
        // Inserted lines stay on the line that follows them in the baseline.
        assert_eq!(diff.base_line(1), 1);
        assert_eq!(diff.base_line(2), 1);
        assert_eq!(diff.base_line(3), 1);
        assert_eq!(diff.base_line(4), 2);
        assert_eq!(diff.base_line(5), 4);
        assert_eq!(diff.base_line(99), 4);
        // Lines appended at the end map to the baseline's last line.
        let appended = LineDiff::compute(
            "a", "a
b
c",
        );
        assert_eq!(appended.base_line(2), 0);
    }
}
