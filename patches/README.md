# Patched dependencies

Copies of dependencies with a fix the pinned version lacks, used through
`[patch]` in the workspace `Cargo.toml`. Drop a copy once Floem pins a
version that has the fix.

## winit-x11

Floem's winit fork (`lapce/winit`, rev `133268de`) at that revision, with
one change in `src/event_loop.rs` (`Device::new`): a device is a pen only if
it has pressure or tilt axes, not just absolute X/Y ones.

Xwayland's pointer, and so the mouse under WSLg's X11, reports `Abs X` and
`Abs Y`. Unpatched, winit takes it for a pen, and its X11 backend only
passes on motion and clicks from mice, so the window got no pointer input at
all. Upstream winit (`rust-windowing/winit`) made the same change.
