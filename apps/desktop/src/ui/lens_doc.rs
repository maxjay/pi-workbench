//! The changes lens inside an editor: removed lines appear as ghost lines in
//! place, changed lines get a bar and a tint, and focus mode folds the rest.
use super::{ACCENT, ADDED, MUTED, REMOVED, REMOVED_TEXT};
use floem::{
    peniko::Color,
    prelude::*,
    reactive::RwSignal,
    text::FamilyOwned,
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
use floem::views::editor::core::xi_rope::Rope;
use std::{
    borrow::Cow,
    collections::BTreeMap,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};
use workbench_core::diff::{LineChange, LineDiff};

/// What the lens adds to one document, rebuilt whenever its diff changes.
#[derive(Clone, Default)]
pub struct Overlay {
    /// Removed baseline text, shown before the current line it was removed from.
    pub ghosts: BTreeMap<usize, String>,
    /// Focus mode: lines folded away, and the count shown where each fold starts.
    pub folded: Vec<bool>,
    pub folds: BTreeMap<usize, usize>,
}

/// Lines kept around each change when focusing.
const CONTEXT: usize = 3;

impl Overlay {
    pub fn new(diff: &LineDiff, base: &str, focus: bool) -> Self {
        let base_lines: Vec<&str> = base.split('\n').map(|l| l.trim_end_matches('\r')).collect();
        let ghosts = diff
            .hunks_iter()
            .filter(|hunk| !hunk.base.is_empty())
            .map(|hunk| {
                let text = base_lines
                    .get(hunk.base.clone())
                    .unwrap_or_default()
                    .join("\n");
                (hunk.current.start, text)
            })
            .collect();
        let (folded, folds) = if focus {
            Self::fold(diff)
        } else {
            (Vec::new(), BTreeMap::new())
        };
        Self {
            ghosts,
            folded,
            folds,
        }
    }

    fn fold(diff: &LineDiff) -> (Vec<bool>, BTreeMap<usize, usize>) {
        let lines = diff.current.len();
        let mut keep = vec![false; lines];
        for hunk in diff.hunks_iter() {
            let start = hunk.current.start.saturating_sub(CONTEXT);
            let end = (hunk.current.end + CONTEXT).min(lines);
            keep[start..end].iter_mut().for_each(|k| *k = true);
        }
        let folded: Vec<bool> = keep.iter().map(|k| !k).collect();
        let mut folds = BTreeMap::new();
        let mut line = 0;
        while line < lines {
            if folded[line] {
                let start = line;
                while line < lines && folded[line] {
                    line += 1;
                }
                folds.insert(start, line - start);
            } else {
                line += 1;
            }
        }
        (folded, folds)
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

fn ghost(col: usize, text: String, fg: Color, bg: Option<Color>) -> PhantomText {
    PhantomText {
        kind: PhantomTextKind::Diagnostic,
        col,
        affinity: None,
        text,
        font_size: None,
        fg: Some(fg),
        bg,
        under_line: None,
    }
}

impl DocumentPhantom for LensDocument {
    fn phantom_text(&self, edid: EditorId, styling: &EditorStyle, line: usize) -> PhantomTextLine {
        let mut text = self.inner.phantom_text(edid, styling, line);
        let overlay = self.overlay.get_untracked();
        let end = self.inner.rope_text().line_end_col(line, true);
        if line == 0
            && let Some(removed) = overlay.ghosts.get(&0)
        {
            text.text.push(ghost(0, format!("{removed}\n"), REMOVED_TEXT, Some(REMOVED)));
        }
        if let Some(removed) = overlay.ghosts.get(&(line + 1)) {
            text.text
                .push(ghost(end, format!("\n{removed}"), REMOVED_TEXT, Some(REMOVED)));
        }
        if let Some(count) = overlay.folds.get(&(line + 1)) {
            let marker = format!("\n··· {count} unchanged lines ···");
            text.text.push(ghost(end, marker, MUTED, None));
        }
        text
    }

    fn has_multiline_phantom(&self, edid: EditorId, styling: &EditorStyle) -> bool {
        let overlay = self.overlay.get_untracked();
        !overlay.ghosts.is_empty()
            || !overlay.folds.is_empty()
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
    fn compute_screen_lines(&self, editor: &Editor, base: RwSignal<ScreenLinesBase>) -> ScreenLines {
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
    lens: Option<Rc<LineDiff>>,
    /// Lines Pi just changed; drawn stronger until the next edit.
    incoming: Option<Rc<LineDiff>>,
    overlay: Rc<Overlay>,
}

impl LensStyling {
    pub fn new(
        code: SimpleStyling,
        lens: Option<Rc<LineDiff>>,
        incoming: Option<Rc<LineDiff>>,
        overlay: Rc<Overlay>,
    ) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            code,
            lens,
            incoming,
            overlay,
        }
    }

    fn folded(&self, line: usize) -> bool {
        self.overlay.folded.get(line).copied().unwrap_or(false)
    }
}

fn added(diff: &Option<Rc<LineDiff>>, line: usize) -> bool {
    diff.as_ref()
        .and_then(|d| d.current.get(line))
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
        if self.folded(line) {
            1
        } else {
            self.code.font_size(edid, line)
        }
    }
    fn line_height(&self, edid: EditorId, line: usize) -> f32 {
        // Folded lines keep a sliver of height so the editor's line maths still works.
        if self.folded(line) {
            1.0
        } else {
            self.code.line_height(edid, line)
        }
    }
    fn apply_layout_styles(
        &self,
        edid: EditorId,
        _style: &EditorStyle,
        line: usize,
        layout_line: &mut TextLayoutLine,
    ) {
        if self.folded(line) {
            return;
        }
        let height = f64::from(self.line_height(edid, line));
        let mark = |x: f64, width: Option<f64>, color: Color| LineExtraStyle {
            x,
            y: 0.0,
            width,
            height,
            bg_color: Some(color),
            under_line: None,
            wave_line: None,
        };
        if added(&self.incoming, line) {
            layout_line.extra_style.push(mark(0.0, None, ADDED));
        } else if added(&self.lens, line) {
            layout_line
                .extra_style
                .push(mark(0.0, None, ADDED.with_alpha(0.08)));
            layout_line.extra_style.push(mark(0.0, Some(3.0), ACCENT));
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
        let overlay = Overlay::new(&diff, base, false);
        assert_eq!(overlay.ghosts.get(&1).map(String::as_str), Some("old 1\nold 2"));
        assert_eq!(overlay.ghosts.len(), 1);
    }

    #[test]
    fn focus_folds_lines_far_from_any_change() {
        let base: String = (0..20).map(|i| format!("{i}\n")).collect();
        let current = base.replace("10\n", "ten\n");
        let diff = LineDiff::compute(&base, &current);
        let overlay = Overlay::new(&diff, &base, true);
        // Lines 7..14 stay: the change at 10 plus three lines either side.
        assert_eq!(overlay.folds.get(&0), Some(&7));
        assert_eq!(overlay.folds.get(&14), Some(&7));
        assert!(!overlay.folded[7] && !overlay.folded[13] && overlay.folded[14]);
    }
}
