//! The host clipboard, shared by every machine: the window system's while a window is open, a buffer otherwise.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// True while a display is open.
static WINDOWED: AtomicBool = AtomicBool::new(false);

/// What a headless run copies into.
static BUFFER: Mutex<String> = Mutex::new(String::new());

/// Records whether the window system is reachable.
pub fn attach(windowed: bool) {
    WINDOWED.store(windowed, Ordering::Relaxed);
}

/// The clipboard's text, empty when nothing has been copied.
pub fn get() -> String {
    if WINDOWED.load(Ordering::Relaxed) {
        if let Some(text) = crate::display::clipboard() {
            return text;
        }
    }
    buffer().clone()
}

pub fn set(text: &str) {
    if WINDOWED.load(Ordering::Relaxed) && crate::display::set_clipboard(text) {
        return;
    }
    buffer().clear();
    buffer().push_str(text);
}

fn buffer() -> std::sync::MutexGuard<'static, String> {
    BUFFER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
