//! One window per machine, presenting its framebuffer and feeding it input.

use crate::events::Event;

#[cfg(sdl)]
mod sdl;
#[cfg(sdl)]
pub use sdl::{clipboard, keys, set_clipboard, Display};

#[cfg(not(sdl))]
mod headless;
#[cfg(not(sdl))]
pub use headless::{clipboard, set_clipboard, Display};

/// A machine's window, as the caller asks for it.
pub struct Spec {
    pub title: String,
    pub width: u32,
    pub height: u32,
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
