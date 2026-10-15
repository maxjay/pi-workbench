//! Three-way merges, so edits from the editor and from disk never need a prompt.
use similar::TextMerge;

#[derive(Debug, PartialEq, Eq)]
pub struct Merged {
    pub text: String,
    /// Regions both sides changed differently; left in place with conflict markers.
    pub conflicts: usize,
}

/// Combines the editor's changes (`ours`) and the disk's (`theirs`) since `base`.
pub fn merge(base: &str, ours: &str, theirs: &str, theirs_label: &str) -> Merged {
    if ours == base || ours == theirs {
        return Merged {
            text: theirs.into(),
            conflicts: 0,
        };
    }
    if theirs == base {
        return Merged {
            text: ours.into(),
            conflicts: 0,
        };
    }
    let mut merge = TextMerge::from_lines(base, ours, theirs);
    merge.labels("before", "editor", theirs_label);
    Merged {
        text: merge.to_string(),
        conflicts: merge.conflict_count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_sides_take_the_other_side() {
        assert_eq!(merge("a\n", "a\n", "b\n", "pi").text, "b\n");
        assert_eq!(merge("a\n", "b\n", "a\n", "pi").text, "b\n");
    }

    #[test]
    fn separate_edits_combine_without_conflicts() {
        let merged = merge(
            "one\ntwo\nthree\n",
            "ONE\ntwo\nthree\n",
            "one\ntwo\nTHREE\n",
            "pi",
        );
        assert_eq!(merged.text, "ONE\ntwo\nTHREE\n");
        assert_eq!(merged.conflicts, 0);
    }

    #[test]
    fn overlapping_edits_keep_both_with_markers() {
        let merged = merge("one\n", "editor\n", "agent\n", "pi");
        assert_eq!(merged.conflicts, 1);
        assert!(merged.text.contains("<<<<<<< editor"));
        assert!(merged.text.contains(">>>>>>> pi"));
        assert!(merged.text.contains("editor\n") && merged.text.contains("agent\n"));
    }
}
