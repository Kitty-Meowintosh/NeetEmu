//! The display in a build without SDL3, where no window ever opens.

use std::convert::Infallible;

use super::{Input, Spec};

pub fn clipboard() -> Option<String> {
    None
}

pub fn set_clipboard(_text: &str) -> bool {
    false
}

/// Never constructed, since `open` always fails.
pub struct Display {
    never: Infallible,
}

impl Display {
    pub fn open(_specs: &[Spec]) -> Result<Display, String> {
        Err("this build has no SDL3; pass --headless".into())
    }

    pub fn pump(&mut self) -> Input {
        match self.never {}
    }

    pub fn present(&mut self, _machine: usize, _pixels: &[u32]) {
        match self.never {}
    }

    pub fn close(&mut self, _machine: usize) {
        match self.never {}
    }

    pub fn is_empty(&self) -> bool {
        match self.never {}
    }
}
