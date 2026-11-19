//! The window's icon, sharp in the title bar and in the taskbar.
//!
//! Windows keeps two icons for a window: a small one for the title bar and
//! a large one. Tauri gives it one picture for both, so Windows shrinks it
//! and the edges go soft. This gives each place a picture of the size it is
//! drawn at, out of the icon the program carries.
//!
//! The taskbar draws the large one, at one and a half times the small size
//! (24 pixels at 100%), not at the size Windows calls large (32). So the
//! large icon is loaded at the taskbar's size.

use tauri::WebviewWindow;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXSMICON, SendMessageW,
    WM_SETICON,
};
use windows::core::PCWSTR;

/// Where the program's own icon is among its resources. Tauri puts it there.
const APP_ICON: usize = 32512;

/// Sets both icons for the screen the window is on. Called again when the
/// window moves to a screen of another scale. The few icons it loads live as
/// long as the program does.
pub fn sharpen(window: &WebviewWindow) {
    let Ok(hwnd) = window.hwnd() else { return };
    // SAFETY: plain calls on this window's own handle and this program's own resources.
    unsafe {
        let Ok(module) = GetModuleHandleW(PCWSTR::null()) else {
            return;
        };
        let small = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForWindow(hwnd));
        for (which, size) in [(ICON_SMALL, small), (ICON_BIG, small * 3 / 2)] {
            let loaded = LoadImageW(
                Some(module.into()),
                PCWSTR(APP_ICON as *const u16),
                IMAGE_ICON,
                size,
                size,
                LR_DEFAULTCOLOR,
            );
            if let Ok(icon) = loaded {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(which as usize)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
    }
}
