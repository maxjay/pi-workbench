//! Line diffs between a baseline and the current text, independent of the renderer.
use similar::{Algorithm, DiffTag, capture_diff_slices_deadline};
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

#[cfg(test)]
mod tests {
    use super::*;
    use LineChange::*;

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
