//! One SDL3 window per machine, presenting its framebuffer and feeding it input.

use std::ffi::{c_char, c_double, c_int, c_uint, c_ushort, c_void, CStr, CString};

use crate::events::Event;
use crate::input::{self, Button, Mouse};

/// Mirrored by `NeetInput` in csrc/display.c.
#[repr(C)]
#[derive(Default)]
struct RawInput {
    kind: c_int,
    window: c_uint,
    code: c_int,
    mods: c_int,
    x: c_int,
    y: c_int,
    dx: c_double,
    dy: c_double,
}

const KEY_DOWN: c_int = 1;
const KEY_UP: c_int = 2;
const MOUSE_MOVED: c_int = 3;
const MOUSE_DOWN: c_int = 4;
const MOUSE_UP: c_int = 5;
const WHEEL: c_int = 6;
const WINDOW_CLOSED: c_int = 7;
const QUIT: c_int = 8;

#[cfg(sdl)]
extern "C" {
    fn neet_display_init() -> c_int;
    fn neet_display_quit();
    fn neet_display_error() -> *const c_char;
    fn neet_clipboard_get() -> *mut c_char;
    fn neet_clipboard_free(text: *mut c_char);
    fn neet_clipboard_set(text: *const c_char) -> c_int;
    fn neet_window_open(title: *const c_char, width: c_int, height: c_int) -> *mut c_void;
    fn neet_window_close(handle: *mut c_void);
    fn neet_window_id(handle: *mut c_void) -> c_uint;
    fn neet_window_present(handle: *mut c_void, pixels: *const c_uint) -> c_int;
    fn neet_display_poll(out: *mut RawInput) -> c_int;
    fn neet_key(key: c_uint, mods: c_ushort) -> c_int;
    fn neet_mods(mods: c_ushort) -> c_int;
    fn neet_button(button: u8) -> c_int;
    fn neet_keycode(name: *const c_char) -> c_uint;
    fn neet_modifier(name: *const c_char) -> c_ushort;
}

/// The mapping csrc/display.c applies, named rather than numbered.
#[cfg(sdl)]
pub mod keys {
    use super::*;

    fn named<T>(name: &str, f: unsafe extern "C" fn(*const c_char) -> T) -> T {
        let name = CString::new(name).expect("key name");
        unsafe { f(name.as_ptr()) }
    }

    /// SDL's keycode for a key SDL names, such as `Return` or `F13`.
    pub fn keycode(name: &str) -> u32 {
        named(name, neet_keycode)
    }

    /// SDL's modifier bits for `shift`, `ctrl`, `alt`, `super`, `caps` or `num`.
    pub fn modifier(name: &str) -> u16 {
        named(name, neet_modifier)
    }

    /// The NEET code for a key, or zero when the event is suppressed.
    pub fn code(key: u32, mods: u16) -> i32 {
        unsafe { neet_key(key, mods) }
    }

    /// The GLFW-shaped modifier bitfield NEET passes through.
    pub fn mods(mods: u16) -> i32 {
        unsafe { neet_mods(mods) }
    }

    /// Minecraft's button number for an SDL button.
    pub fn button(button: u8) -> i32 {
        unsafe { neet_button(button) }
    }
}

/// The window system's clipboard, or None when this build has no SDL3.
#[cfg(sdl)]
pub fn clipboard() -> Option<String> {
    unsafe {
        let text = neet_clipboard_get();
        if text.is_null() {
            return None;
        }
        let out = CStr::from_ptr(text).to_string_lossy().into_owned();
        neet_clipboard_free(text);
        Some(out)
    }
}

#[cfg(not(sdl))]
pub fn clipboard() -> Option<String> {
    None
}

/// Puts `text` on the window system's clipboard, reporting whether it took.
#[cfg(sdl)]
pub fn set_clipboard(text: &str) -> bool {
    let Ok(text) = CString::new(text) else {
        return false;
    };
    unsafe { neet_clipboard_set(text.as_ptr()) == 0 }
}

#[cfg(not(sdl))]
pub fn set_clipboard(_text: &str) -> bool {
    false
}

/// A machine's window, as the caller asks for it.
pub struct Spec {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

struct Window {
    handle: *mut c_void,
    id: c_uint,
    width: i32,
    height: i32,
    mouse: Mouse,
}

/// What a pump produced, addressed by the machine's index.
#[derive(Default)]
pub struct Input {
    pub events: Vec<(usize, Event)>,
    /// Machines whose window the user closed.
    pub closed: Vec<usize>,
    /// The user quit the emulator itself.
    pub quit: bool,
}

pub struct Display {
    windows: Vec<Option<Window>>,
}

impl Display {
    /// Opens one window per spec, in the same order.
    #[cfg(sdl)]
    pub fn open(specs: &[Spec]) -> Result<Display, String> {
        unsafe {
            if neet_display_init() != 0 {
                return Err(format!("SDL3: {}", last_error()));
            }
            let mut windows = Vec::with_capacity(specs.len());
            for spec in specs {
                let title = CString::new(spec.title.as_str()).map_err(|e| e.to_string())?;
                let handle =
                    neet_window_open(title.as_ptr(), spec.width as c_int, spec.height as c_int);
                if handle.is_null() {
                    let error = last_error();
                    for window in windows.iter().flatten() {
                        let window: &Window = window;
                        neet_window_close(window.handle);
                    }
                    neet_display_quit();
                    return Err(format!("SDL3: {error}"));
                }
                windows.push(Some(Window {
                    handle,
                    id: neet_window_id(handle),
                    width: spec.width as i32,
                    height: spec.height as i32,
                    mouse: Mouse::new(),
                }));
            }
            crate::clipboard::attach(true);
            Ok(Display { windows })
        }
    }

    #[cfg(not(sdl))]
    pub fn open(_specs: &[Spec]) -> Result<Display, String> {
        Err("this build has no SDL3; pass --headless".into())
    }

    /// Drains the window system, translating what it reports into NEET events.
    pub fn pump(&mut self) -> Input {
        let mut input = Input::default();
        #[cfg(sdl)]
        loop {
            let mut raw = RawInput::default();
            if unsafe { neet_display_poll(&mut raw) } == 0 {
                break;
            }
            if raw.kind == QUIT {
                input.quit = true;
                continue;
            }
            let Some(machine) = self.machine_of(raw.window) else {
                continue;
            };
            if raw.kind == WINDOW_CLOSED {
                input.closed.push(machine);
                continue;
            }
            let Some(window) = self.windows[machine].as_mut() else {
                continue;
            };
            window.handle(&raw, machine, &mut input.events);
        }
        for (machine, window) in self.windows.iter_mut().enumerate() {
            let Some(window) = window else { continue };
            for event in window.mouse.flush() {
                input.events.push((machine, event));
            }
        }
        input
    }

    /// Puts a machine's framebuffer on screen.
    pub fn present(&mut self, machine: usize, pixels: &[u32]) {
        let Some(Some(_window)) = self.windows.get(machine) else {
            return;
        };
        #[cfg(sdl)]
        unsafe {
            neet_window_present(_window.handle, pixels.as_ptr());
        }
        let _ = pixels;
    }

    /// Closes one machine's window, leaving the rest open.
    pub fn close(&mut self, machine: usize) {
        if let Some(slot) = self.windows.get_mut(machine) {
            if let Some(_window) = slot.take() {
                #[cfg(sdl)]
                unsafe {
                    neet_window_close(_window.handle);
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.windows.iter().all(Option::is_none)
    }

    fn machine_of(&self, id: c_uint) -> Option<usize> {
        self.windows
            .iter()
            .position(|w| w.as_ref().is_some_and(|w| w.id == id))
    }
}

impl Window {
    /// The framebuffer's bounds, outside which no pointer events are sent.
    fn inside(&self, x: c_int, y: c_int) -> bool {
        x >= 0 && y >= 0 && x < self.width && y < self.height
    }

    fn handle(&mut self, raw: &RawInput, machine: usize, out: &mut Vec<(usize, Event)>) {
        let inside = self.inside(raw.x, raw.y);
        if raw.kind != KEY_DOWN && raw.kind != KEY_UP {
            self.mouse.modifiers(raw.mods);
        }
        match raw.kind {
            KEY_DOWN | KEY_UP => {
                if let Some(event) = input::key(raw.kind == KEY_DOWN, raw.code, raw.mods) {
                    out.push((machine, event));
                }
            }
            MOUSE_MOVED => {
                if inside {
                    self.mouse.moved(raw.x, raw.y);
                } else {
                    out.extend(self.mouse.left().into_iter().map(|e| (machine, e)));
                }
            }
            MOUSE_DOWN if inside => {
                let event = self.mouse.press(raw.x, raw.y, Button::from_code(raw.code));
                out.push((machine, event));
            }
            MOUSE_UP if inside => {
                let event = self
                    .mouse
                    .release(raw.x, raw.y, Button::from_code(raw.code));
                out.push((machine, event));
            }
            WHEEL if inside => {
                out.push((
                    machine,
                    input::wheel(raw.x, raw.y, raw.dx, raw.dy, raw.mods),
                ));
            }
            _ => {}
        }
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        crate::clipboard::attach(false);
        #[cfg(sdl)]
        unsafe {
            for window in self.windows.iter().flatten() {
                neet_window_close(window.handle);
            }
            neet_display_quit();
        }
    }
}

#[cfg(sdl)]
unsafe fn last_error() -> String {
    CStr::from_ptr(neet_display_error())
        .to_string_lossy()
        .into_owned()
}
