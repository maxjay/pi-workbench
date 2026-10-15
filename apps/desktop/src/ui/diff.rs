//! Side-by-side changes: the read-only baseline on the left, the editable document on the right.
use super::{ADDED, BORDER, MUTED, PANEL, REMOVED, button};
use crate::{
    changes::{Comparison, summary},
    workspace::Workbench,
};
use floem::{
    kurbo::Vec2,
    prelude::*,
    reactive::{Effect, ReadSignal},
    views::{
        dyn_container,
        editor::{
            EditorStyle,
            id::EditorId,
            layout::{LineExtraStyle, TextLayoutLine},
            text::{Styling, WrapMethod, default_dark_color},
            text_document::TextDocument,
        },
    },
};
use std::{
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};
use workbench_core::diff::{LineChange, LineDiff};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Base,
    Current,
}

/// Default editor styling plus diff line backgrounds.
struct DiffStyling {
    id: u64,
    diff: Rc<LineDiff>,
    side: Side,
}

impl DiffStyling {
    fn new(diff: Rc<LineDiff>, side: Side) -> Self {
        // Floem caches line layouts by styling id, so each diff needs a fresh one.
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            diff,
            side,
        }
    }
}

impl Styling for DiffStyling {
    fn id(&self) -> u64 {
        self.id
    }

    fn apply_layout_styles(
        &self,
        edid: EditorId,
        _style: &EditorStyle,
        line: usize,
        layout_line: &mut TextLayoutLine,
    ) {
        let lines = match self.side {
            Side::Base => &self.diff.base,
            Side::Current => &self.diff.current,
        };
        let color = match lines.get(line) {
            Some(LineChange::Removed) => REMOVED,
            Some(LineChange::Added) => ADDED,
            _ => return,
        };
        layout_line.extra_style.push(LineExtraStyle {
            x: 0.0,
            y: 0.0,
            width: None,
            height: self.line_height(edid, line).into(),
            bg_color: Some(color),
            under_line: None,
            wave_line: None,
        });
    }
}

fn side_editor(doc: Rc<TextDocument>, diff: ReadSignal<Rc<LineDiff>>, side: Side) -> TextEditor {
    let editor = text_editor("")
        .editor_style(default_dark_color)
        // Unwrapped lines keep both sides on the same line height for scroll sync.
        .editor_style(|s| {
            s.scroll_beyond_last_line(false)
                .wrap_method(WrapMethod::None)
        })
        .style(|s| s.size_full().min_width(0).min_height(0));
    let editor = if side == Side::Base {
        editor.read_only()
    } else {
        editor
    };
    let handle = editor.editor().clone();
    handle.update_doc(doc, None);
    Effect::new(move |_| {
        let diff = diff.get();
        handle.update_styling(Rc::new(DiffStyling::new(diff, side)));
    });
    editor
}

fn column(title: impl Fn() -> String + 'static, editor: TextEditor) -> impl IntoView {
    Stack::vertical((
        Label::derived(title).style(|s| {
            s.color(MUTED)
                .font_size(11)
                .height(28)
                .padding_horiz(12)
                .items_center()
                .width_full()
                .text_ellipsis()
                .flex_shrink(0.0_f32)
        }),
        editor,
    ))
    .style(|s| {
        s.flex_basis(0)
            .flex_grow(1.0_f32)
            .height_full()
            .min_width(0)
            .min_height(0)
    })
}

fn comparison(state: Rc<Workbench>, comparison: Rc<Comparison>) -> impl IntoView {
    let diff = comparison.diff();
    let base = side_editor(comparison.base.clone(), diff, Side::Base);
    let current = side_editor(comparison.document.text(), diff, Side::Current);

    // The working copy drives the baseline, keeping corresponding lines level.
    let base_editor = base.editor().clone();
    let current_editor = current.editor().clone();
    Effect::new(move |_| {
        let viewport = current_editor.viewport.get();
        let line_height = f64::from(current_editor.line_height(0));
        let top = viewport.y0 / line_height;
        let line = top.floor() as usize;
        let base_line = diff.with(|d| d.base_line(line));
        base_editor.scroll_to.set(Some(Vec2::new(
            viewport.x0,
            (base_line as f64 + top.fract()) * line_height,
        )));
    });

    let relative = comparison
        .path()
        .strip_prefix(state.root())
        .unwrap_or(comparison.path())
        .display()
        .to_string();
    let label = comparison.label.clone();
    let base_title = format!("{label} · read-only");
    let document = comparison.document.clone();
    let header = Stack::horizontal((
        Label::new(relative.clone()).style(|s| s.min_width(0).text_ellipsis()),
        Label::derived(move || diff.with(|d| summary(d, &label))).style(|s| {
            s.color(MUTED)
                .flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
        }),
        button("Close", move || state.changes.close()),
    ))
    .style(|s| {
        s.width_full()
            .height(40)
            .padding_horiz(12)
            .gap(12)
            .items_center()
            .flex_shrink(0.0_f32)
            .background(PANEL)
            .border_bottom(1)
            .border_color(BORDER)
    });
    Stack::vertical((
        header,
        Stack::horizontal((
            column(move || base_title.clone(), base),
            Empty::new().style(|s| s.width(5).height_full().background(BORDER)),
            column(
                move || {
                    format!(
                        "Working copy{}",
                        if document.dirty() { " • unsaved" } else { "" }
                    )
                },
                current,
            ),
        ))
        .style(|s| s.size_full().min_width(0).min_height(0)),
    ))
    .style(|s| s.size_full().min_width(0).min_height(0))
}

pub(super) fn changes(state: Rc<Workbench>) -> impl IntoView {
    let current = state.changes.current();
    dyn_container(
        move || current.get(),
        move |value| match value {
            Some(value) => comparison(state.clone(), value).into_any(),
            None => Empty::new().into_any(),
        },
    )
    .style(move |s| {
        s.size_full()
            .min_width(0)
            .min_height(0)
            .apply_if(current.with(Option::is_none), |s| s.hide())
    })
}
