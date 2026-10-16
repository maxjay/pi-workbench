//! The popped-out pet's window on Windows: a layered window drawn from
//! premultiplied pixels, so only the pet shows on the desktop and clicks on
//! the clear parts fall through to whatever is below. Floem's surfaces are
//! opaque, so this one is drawn by hand.
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::Gdi::{
        AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
        CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
        ReleaseDC, SelectObject,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture},
        WindowsAndMessaging::{
            AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
            GetCursorPos, GetWindowRect, HTCAPTION, MA_NOACTIVATE, MF_STRING, PostMessageW,
            RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SendMessageW, SetForegroundWindow,
            ShowWindow, TPM_NONOTIFY, TPM_RETURNCMD, TrackPopupMenu, ULW_ALPHA,
            UpdateLayeredWindow, WM_CLOSE, WM_EXITSIZEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP,
            WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOVING, WM_NCDESTROY, WM_NCLBUTTONDOWN,
            WM_RBUTTONUP, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
            WS_EX_TOPMOST, WS_POPUP,
        },
    },
};

/// What the window tells the pet about.
pub trait Events {
    /// Clicked without moving.
    fn clicked(&self);
    /// Being dragged; `dx` is how far it moved sideways just now.
    fn dragged(&self, dx: i32);
    /// Let go after a drag.
    fn dropped(&self);
    /// A choice from the right-click menu, by its index in `menu`.
    fn chose(&self, index: usize);
    /// The right-click menu's items.
    fn menu(&self) -> Vec<&'static str>;
}

struct Entry {
    events: Rc<dyn Events>,
    /// Pressed here (screen point), and not yet moved far enough to drag.
    pressed: Option<(i32, i32)>,
    /// Left edge at the last move, to tell which way it is dragged.
    left: i32,
}

thread_local! {
    static WINDOWS: RefCell<HashMap<isize, Entry>> = RefCell::new(HashMap::new());
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn cursor() -> (i32, i32) {
    let mut at = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut at) };
    (at.x, at.y)
}

/// The events for a window, taken out so the map is free while they run.
fn events(hwnd: HWND) -> Option<Rc<dyn Events>> {
    WINDOWS.with_borrow(|windows| windows.get(&(hwnd as isize)).map(|e| e.events.clone()))
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let key = hwnd as isize;
    match msg {
        // The pet never takes the keyboard from what you are doing.
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
        WM_LBUTTONDOWN => {
            let at = cursor();
            WINDOWS.with_borrow_mut(|w| {
                if let Some(entry) = w.get_mut(&key) {
                    entry.pressed = Some(at);
                }
            });
            unsafe { SetCapture(hwnd) };
            return 0;
        }
        WM_MOUSEMOVE => {
            let at = cursor();
            let drag = WINDOWS.with_borrow_mut(|w| {
                let entry = w.get_mut(&key)?;
                let (x, y) = entry.pressed?;
                ((at.0 - x).abs() + (at.1 - y).abs() > 3).then(|| entry.pressed = None)
            });
            if drag.is_some() {
                // The system moves the window from here until the button is up.
                unsafe {
                    ReleaseCapture();
                    SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as WPARAM, 0);
                }
            }
            return 0;
        }
        WM_LBUTTONUP => {
            let clicked =
                WINDOWS.with_borrow_mut(|w| w.get_mut(&key).and_then(|e| e.pressed.take()));
            unsafe { ReleaseCapture() };
            if clicked.is_some()
                && let Some(events) = events(hwnd)
            {
                events.clicked();
            }
            return 0;
        }
        WM_MOVING => {
            let rect = unsafe { &*(lparam as *const RECT) };
            let dx = WINDOWS.with_borrow_mut(|w| {
                let entry = w.get_mut(&key)?;
                let dx = rect.left - entry.left;
                entry.left = rect.left;
                Some(dx)
            });
            if let Some(dx) = dx
                && dx != 0
                && let Some(events) = events(hwnd)
            {
                events.dragged(dx);
            }
        }
        WM_EXITSIZEMOVE => {
            if let Some(events) = events(hwnd) {
                events.dropped();
            }
        }
        WM_RBUTTONUP => {
            if let Some(events) = events(hwnd) {
                let items = events.menu();
                let (x, y) = cursor();
                let chosen = unsafe {
                    let menu = CreatePopupMenu();
                    for (index, item) in items.iter().enumerate() {
                        AppendMenuW(menu, MF_STRING, index + 1, wide(item).as_ptr());
                    }
                    // Without this the menu does not close when you click away.
                    SetForegroundWindow(hwnd);
                    let chosen = TrackPopupMenu(
                        menu,
                        TPM_RETURNCMD | TPM_NONOTIFY,
                        x,
                        y,
                        0,
                        hwnd,
                        std::ptr::null(),
                    );
                    DestroyMenu(menu);
                    chosen
                };
                if chosen > 0 {
                    events.chose(chosen as usize - 1);
                }
            }
            return 0;
        }
        WM_NCDESTROY => {
            WINDOWS.with_borrow_mut(|w| w.remove(&key));
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

const CLASS: &str = "PiWorkbenchPet";

fn register() {
    thread_local! {
        static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if DONE.replace(true) {
        return;
    }
    let class = wide(CLASS);
    let info = WNDCLASSW {
        lpfnWndProc: Some(procedure),
        hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
        lpszClassName: class.as_ptr(),
        ..Default::default()
    };
    unsafe { RegisterClassW(&info) };
}

/// A see-through window on top of everything, showing whatever pixels it
/// is given.
pub struct Layered {
    hwnd: HWND,
}

impl Layered {
    /// Opens at a screen point (physical pixels).
    pub fn open(at: (i32, i32), events: Rc<dyn Events>) -> Option<Self> {
        register();
        let class = wide(CLASS);
        let title = wide("Glitch");
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                at.0,
                at.1,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        WINDOWS.with_borrow_mut(|w| {
            w.insert(
                hwnd as isize,
                Entry {
                    events,
                    pressed: None,
                    left: at.0,
                },
            )
        });
        unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        Some(Self { hwnd })
    }

    /// The screen's pixels per logical pixel where the window is.
    pub fn scale(&self) -> f64 {
        f64::from(unsafe { GetDpiForWindow(self.hwnd) }.max(96)) / 96.0
    }

    /// Where the window is on the screen, and its size (physical pixels).
    pub fn rect(&self) -> (i32, i32, i32, i32) {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe { GetWindowRect(self.hwnd, &mut rect) };
        (
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        )
    }

    /// Shows straight-alpha RGBA pixels, moving the window to `at` if given.
    pub fn draw(&self, rgba: &[u8], width: u32, height: u32, at: Option<(i32, i32)>) {
        if rgba.len() < (width * height * 4) as usize || width == 0 || height == 0 {
            return;
        }
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let memory = CreateCompatibleDC(screen);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    // Negative: rows from the top down.
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                memory,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            if !bitmap.is_null() && !bits.is_null() {
                let out = std::slice::from_raw_parts_mut(bits as *mut u8, rgba.len());
                // Premultiplied BGRA, as layered windows take it.
                for (from, to) in rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(out.as_chunks_mut::<4>().0)
                {
                    let alpha = u16::from(from[3]);
                    let premultiply = |c: u8| ((u16::from(c) * alpha + 127) / 255) as u8;
                    to[0] = premultiply(from[2]);
                    to[1] = premultiply(from[1]);
                    to[2] = premultiply(from[0]);
                    to[3] = from[3];
                }
                let old = SelectObject(memory, bitmap);
                let size = SIZE {
                    cx: width as i32,
                    cy: height as i32,
                };
                let source = POINT { x: 0, y: 0 };
                let place = at.map(|(x, y)| POINT { x, y });
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                UpdateLayeredWindow(
                    self.hwnd,
                    screen,
                    place
                        .as_ref()
                        .map_or(std::ptr::null(), |p| p as *const POINT),
                    &size,
                    memory,
                    &source,
                    0,
                    &blend,
                    ULW_ALPHA,
                );
                if let Some((x, _)) = at {
                    WINDOWS.with_borrow_mut(|w| {
                        if let Some(entry) = w.get_mut(&(self.hwnd as isize)) {
                            entry.left = x;
                        }
                    });
                }
                SelectObject(memory, old);
                DeleteObject(bitmap);
            }
            DeleteDC(memory);
            ReleaseDC(std::ptr::null_mut(), screen);
        }
    }
}

impl Drop for Layered {
    /// Hidden now and closed on the next turn of the message loop, as this
    /// may run inside the window's own message handling.
    fn drop(&mut self) {
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
            PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
        }
    }
}

/// The pointer's place on the screen (physical pixels).
pub fn pointer() -> (i32, i32) {
    cursor()
}
