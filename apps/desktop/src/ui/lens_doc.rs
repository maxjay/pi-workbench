//! The changes lens inside an editor: removed lines appear as ghost lines in
//! place, changed lines get a bar and a tint, and focus mode dims the rest.
use super::{ADDED, MUTED, REMOVED, REMOVED_TEXT};
use crate::{lens::Comparison, origins::Origin, syntax::Lines};
use floem::views::editor::core::xi_rope::Rope;
use floem::{
    peniko::Color,
    prelude::*,
    reactive::RwSignal,
    text::{Attrs, AttrsList, FamilyOwned},
    views::editor::{
        Editor, EditorStyle,
        command::{Command, CommandExecuted},
        core::{
            buffer::rope_text::{RopeText, RopeTextVal},
            editor::EditType,
            selection::Selection,
        },
        id::EditorId,
        layout::{LineExtraStyle, TextLayoutLine},
        phantom_text::{PhantomText, PhantomTextKind, PhantomTextLine},
        text::{Document, DocumentPhantom, PreeditData, SimpleStyling, Styling},
        text_document::TextDocument,
        view::{ScreenLines, ScreenLinesBase},
    },
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};
use workbench_core::diff::{LineChange, LineDiff};

/// Lines kept bright around each change when focusing.
const CONTEXT: usize = 3;

/// What the lens adds to one document, rebuilt whenever its diff changes.
#[derive(Clone, Default)]
pub struct Overlay {
    /// Removed baseline text, shown before the current line it was removed from.
    pub ghosts: BTreeMap<usize, String>,
    /// Focus mode: lines far from any unseen change, drawn faint.
    pub dimmed: Vec<bool>,
    /// Lines of changes marked seen or hidden as formatting: no tint and no ghosts.
    pub quiet: Vec<bool>,
    /// Lines of hidden formatting changes, which keep a faint mark.
    pub faint: Vec<bool>,
}

/// Why a hunk is out of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quiet {
    Seen,
    Formatting,
}

impl Overlay {
    /// `quiet` says, by position, which of the diff's hunks are out of view.
    pub fn new(
        diff: &LineDiff,
        base: &str,
        focus: bool,
        quiet_hunk: impl Fn(usize) -> Option<Quiet>,
    ) -> Self {
        let base_lines: Vec<&str> = base.split('\n').map(|l| l.trim_end_matches('\r')).collect();
        let mut unseen = Vec::new();
        let lines = diff.current.len();
        let mut quiet = vec![false; lines];
        let mut faint = vec![false; lines];
        for (index, hunk) in diff.hunks_iter().enumerate() {
            let Some(why) = quiet_hunk(index) else {
                unseen.push(hunk);
                continue;
            };
            for line in hunk.current.clone().filter(|&line| line < lines) {
                quiet[line] = true;
                faint[line] = why == Quiet::Formatting;
            }
        }
        let ghosts = unseen
            .iter()
            .filter(|hunk| !hunk.base.is_empty())
            .map(|hunk| {
                let text = base_lines
                    .get(hunk.base.clone())
                    .unwrap_or_default()
                    .join("\n");
                (hunk.current.start, text)
            })
            .collect();
        let dimmed = if focus {
            let mut keep = vec![false; lines];
            for hunk in &unseen {
                let start = hunk.current.start.saturating_sub(CONTEXT);
                let end = (hunk.current.end + CONTEXT).min(lines);
                keep[start..end].iter_mut().for_each(|k| *k = true);
            }
            keep.iter().map(|k| !k).collect()
        } else {
            Vec::new()
        };
        Self {
            ghosts,
            dimmed,
            quiet,
            faint,
        }
    }
}

/// Wraps a shared buffer so one editor can show the lens without changing
/// the buffer other editors see.
pub struct LensDocument {
    inner: Rc<TextDocument>,
    overlay: RwSignal<Rc<Overlay>>,
}

impl LensDocument {
    pub fn new(inner: Rc<TextDocument>, overlay: RwSignal<Rc<Overlay>>) -> Self {
        Self { inner, overlay }
    }
}

fn ghost(col: usize, text: String) -> PhantomText {
    PhantomText {
        kind: PhantomTextKind::Diagnostic,
        col,
        affinity: None,
        text,
        font_size: None,
        fg: Some(REMOVED_TEXT),
        bg: Some(REMOVED),
        under_line: None,
    }
}

impl DocumentPhantom for LensDocument {
    fn phantom_text(&self, edid: EditorId, styling: &EditorStyle, line: usize) -> PhantomTextLine {
        let mut text = self.inner.phantom_text(edid, styling, line);
        let overlay = self.overlay.get_untracked();
        // Ghosts go before the line they were removed from: trailing the line
        // above would overlap the next line once the editor wraps.
        if let Some(removed) = overlay.ghosts.get(&line) {
            text.text.push(ghost(0, format!("{removed}\n")));
        }
        // Lines removed from the end have no line after them to sit before.
        if line == self.inner.rope_text().last_line()
            && let Some(removed) = overlay.ghosts.get(&(line + 1))
        {
            let end = self.inner.rope_text().line_end_col(line, true);
            text.text.push(ghost(end, format!("\n{removed}")));
        }
        text
    }

    fn has_multiline_phantom(&self, edid: EditorId, styling: &EditorStyle) -> bool {
        !self.overlay.get_untracked().ghosts.is_empty()
            || self.inner.has_multiline_phantom(edid, styling)
    }
}

impl Document for LensDocument {
    fn text(&self) -> Rope {
        self.inner.text()
    }
    fn rope_text(&self) -> RopeTextVal {
        self.inner.rope_text()
    }
    fn cache_rev(&self) -> RwSignal<u64> {
        self.inner.cache_rev()
    }
    fn find_unmatched(&self, offset: usize, previous: bool, ch: char) -> usize {
        self.inner.find_unmatched(offset, previous, ch)
    }
    fn find_matching_pair(&self, offset: usize) -> usize {
        self.inner.find_matching_pair(offset)
    }
    fn preedit(&self) -> PreeditData {
        self.inner.preedit()
    }
    fn preedit_phantom(&self, under_line: Option<Color>, line: usize) -> Option<PhantomText> {
        self.inner.preedit_phantom(under_line, line)
    }
    fn compute_screen_lines(
        &self,
        editor: &Editor,
        base: RwSignal<ScreenLinesBase>,
    ) -> ScreenLines {
        self.inner.compute_screen_lines(editor, base)
    }
    fn run_command(
        &self,
        ed: &Editor,
        cmd: &Command,
        count: Option<usize>,
        modifiers: Modifiers,
    ) -> CommandExecuted {
        self.inner.run_command(ed, cmd, count, modifiers)
    }
    fn receive_char(&self, ed: &Editor, c: &str) {
        self.inner.receive_char(ed, c)
    }
    fn edit_single(&self, selection: Selection, content: &str, edit_type: EditType) {
        self.inner.edit_single(selection, content, edit_type)
    }
    fn edit(&self, iter: &mut dyn Iterator<Item = (Selection, &str)>, edit_type: EditType) {
        self.inner.edit(iter, edit_type)
    }
}

/// Code styling plus the lens's line marks. Each instance has a fresh id,
/// because Floem caches line layouts by styling id.
pub struct LensStyling {
    id: u64,
    code: SimpleStyling,
    /// Lines changed against the lens baseline.
    lens: Option<Rc<Comparison>>,
    /// Lines Pi just changed; drawn stronger until the next edit.
    incoming: Option<Rc<LineDiff>>,
    overlay: Rc<Overlay>,
    /// Highlight spans, possibly a few edits old, and the text to clamp them to.
    syntax: Option<Lines>,
    text: Rc<TextDocument>,
}

impl LensStyling {
    pub fn new(
        code: SimpleStyling,
        lens: Option<Rc<Comparison>>,
        incoming: Option<Rc<LineDiff>>,
        overlay: Rc<Overlay>,
        syntax: Option<Lines>,
        text: Rc<TextDocument>,
    ) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            code,
            lens,
            incoming,
            overlay,
            syntax,
            text,
        }
    }

    fn flag(lines: &[bool], line: usize) -> bool {
        lines.get(line).copied().unwrap_or(false)
    }
}

fn added(diff: Option<&LineDiff>, line: usize) -> bool {
    diff.and_then(|d| d.current.get(line))
        .is_some_and(|c| *c == LineChange::Added)
}

impl Styling for LensStyling {
    fn id(&self) -> u64 {
        self.id
    }
    fn font_family(&self, edid: EditorId, line: usize) -> Cow<'_, [FamilyOwned]> {
        self.code.font_family(edid, line)
    }
    fn font_size(&self, edid: EditorId, line: usize) -> usize {
        self.code.font_size(edid, line)
    }
    fn line_height(&self, edid: EditorId, line: usize) -> f32 {
        self.code.line_height(edid, line)
    }
    fn apply_attr_styles(
        &self,
        _edid: EditorId,
        _style: &EditorStyle,
        line: usize,
        default: Attrs,
        attrs: &mut AttrsList,
    ) {
        // Floem's editor needs every line the same height, so focus fades
        // unchanged lines instead of folding them.
        if Self::flag(&self.overlay.dimmed, line) {
            *attrs = AttrsList::new(default.color(MUTED.with_alpha(0.3)));
            return;
        }
        let Some(spans) = self.syntax.as_ref().and_then(|lines| lines.get(line)) else {
            return;
        };
        // Spans come from an earlier snapshot; never let one run past the line.
        let length = self.text.rope_text().line_end_col(line, true);
        // Floem styles the line with its phantom text already inserted, so
        // spans move past a ghost placed before the line.
        let shift = self.overlay.ghosts.get(&line).map_or(0, |g| g.len() + 1);
        for span in spans {
            let end = span.end.min(length);
            if span.start >= end {
                continue;
            }
            let (start, end) = (span.start + shift, end + shift);
            if let Some((color, italic)) = super::token_style(span.token) {
                let mut style = default.clone().color(color);
                if italic {
                    style = style.font_style(floem::text::FontStyle::Italic);
                }
                attrs.add_span(start..end, style);
            }
        }
    }
    fn apply_layout_styles(
        &self,
        edid: EditorId,
        _style: &EditorStyle,
        line: usize,
        layout_line: &mut TextLayoutLine,
    ) {
        let faint = Self::flag(&self.overlay.faint, line);
        if Self::flag(&self.overlay.quiet, line) && !faint {
            return;
        }
        let row = f64::from(self.line_height(edid, line));
        // Ghost text shares the line's layout: mark only the rows of the line's
        // own text, which can wrap, between ghosts before and after it.
        let text = &layout_line.text;
        let before = self.overlay.ghosts.get(&line).map_or(0, |g| g.len() + 1);
        let after = self
            .overlay
            .ghosts
            .get(&(line + 1))
            .filter(|_| layout_line.phantom_text.text.iter().any(|p| p.col > 0))
            .map_or(0, |g| g.len() + 1);
        let own = before..text.text().len().saturating_sub(after).max(before + 1);
        let rows: Vec<usize> = (0..text.visual_line_count())
            .filter(|&n| {
                text.visual_line_text_range(n)
                    .is_some_and(|r| r.start < own.end && r.end.max(r.start + 1) > own.start)
            })
            .collect();
        let first = rows.first().copied().unwrap_or(0);
        let count = rows.len().max(1);
        let (y, height) = (first as f64 * row, count as f64 * row);
        let mark = |width: Option<f64>, color: Color| LineExtraStyle {
            x: 0.0,
            y,
            width,
            height,
            bg_color: Some(color),
            under_line: None,
            wave_line: None,
        };
        if faint {
            // Reformatted only: a hairline, so it can be found but not read as a change.
            layout_line
                .extra_style
                .push(mark(Some(1.0), MUTED.with_alpha(0.5)));
        } else if added(self.incoming.as_deref(), line) {
            layout_line.extra_style.push(mark(None, ADDED));
        } else if added(self.lens.as_ref().map(|c| &c.diff), line) {
            let origin = self
                .lens
                .as_ref()
                .and_then(|c| c.origins.get(line).copied().flatten());
            let color = super::origin_color(origin.unwrap_or(Origin::You));
            layout_line
                .extra_style
                .push(mark(None, color.with_alpha(0.08)));
            layout_line.extra_style.push(mark(Some(3.0), color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghosts_sit_before_the_line_they_were_removed_from() {
        let base = "a\nold 1\nold 2\nb\nc";
        let diff = LineDiff::compute(base, "a\nnew\nb\nc\nd");
        let overlay = Overlay::new(&diff, base, false, |_| None);
        assert_eq!(
            overlay.ghosts.get(&1).map(String::as_str),
            Some("old 1\nold 2")
        );
        assert_eq!(overlay.ghosts.len(), 1);
    }

    #[test]
    fn focus_dims_lines_far_from_unseen_changes() {
        let base: String = (0..20).map(|i| format!("{i}\n")).collect();
        let current = base
            .replace("\n10\n", "\nten\n")
            .replace("\n2\n", "\ntwo\n");
        let diff = LineDiff::compute(&base, &current);
        // The change at line 2 is seen, so only line 10 keeps lines bright.
        let overlay = Overlay::new(&diff, &base, true, |index| {
            (index == 0).then_some(Quiet::Seen)
        });
        assert!(overlay.dimmed[2] && overlay.dimmed[6]);
        assert!(!overlay.dimmed[7] && !overlay.dimmed[10] && !overlay.dimmed[13]);
        assert!(overlay.dimmed[14]);
        assert!(overlay.quiet[2] && !overlay.quiet[10]);
    }
}
