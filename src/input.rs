//! Keyboard and mouse in the shape NEET queues them, ported from `graphics/screens/BoilerplateScreen.java`.

use crate::events::{Event, Value};

/// What a window reported, before it becomes an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
    Other(i32),
}

impl Button {
    /// Minecraft numbers buttons from zero, left-right-middle.
    pub fn code(self) -> i64 {
        match self {
            Button::Left => 0,
            Button::Right => 1,
            Button::Middle => 2,
            Button::Other(n) => n as i64,
        }
    }

    pub fn from_code(code: i32) -> Button {
        match code {
            0 => Button::Left,
            1 => Button::Right,
            2 => Button::Middle,
            other => Button::Other(other),
        }
    }
}

/// `keyPressed`/`keyReleased`, or nothing when the key maps to zero.
pub fn key(down: bool, code: i32, mods: i32) -> Option<Event> {
    if code == 0 {
        return None;
    }
    let name = if down { "keyPressed" } else { "keyReleased" };
    Some(Event::new(
        name,
        vec![
            Value::Int(code as i64),
            Value::Str(character(code)),
            Value::Int(mods as i64),
        ],
    ))
}

/// Non-empty only for `code > 31 && code < 128`.
fn character(code: i32) -> String {
    if (32..=127).contains(&code) {
        char::from(code as u8).to_string()
    } else {
        String::new()
    }
}

/// `mouseScrolled`, carrying both axes as floats and the modifier bits held.
pub fn wheel(x: i32, y: i32, dx: f64, dy: f64, mods: i32) -> Event {
    Event::new(
        "mouseScrolled",
        vec![
            Value::Int(x as i64),
            Value::Int(y as i64),
            Value::Num(dx),
            Value::Num(dy),
            Value::Int(mods as i64),
        ],
    )
}

fn at(name: &str, x: i32, y: i32) -> Event {
    Event::new(name, vec![Value::Int(x as i64), Value::Int(y as i64)])
}

fn at_button(name: &str, x: i32, y: i32, button: Button, mods: i32) -> Event {
    Event::new(
        name,
        vec![
            Value::Int(x as i64),
            Value::Int(y as i64),
            Value::Int(button.code()),
            Value::Int(mods as i64),
        ],
    )
}

/// The pointer, reported once per tick and only when it moved, as in `BoilerplateScreen`.
#[derive(Default)]
pub struct Mouse {
    /// Where the pointer is now, or `None` while it is outside the framebuffer.
    at: Option<(i32, i32)>,
    /// The last position `mouseMoved` reported, as `dragTable[-1]`.
    moved: Option<(i32, i32)>,
    /// Each held button and the last position it was reported at, as `dragTable[key]`.
    held: Vec<(Button, (i32, i32))>,
    /// The modifier bits held, as `keyPressed` carries them.
    mods: i32,
}

impl Mouse {
    pub fn new() -> Mouse {
        Mouse::default()
    }

    /// The modifier bits held now, sent with every button event after it.
    pub fn modifiers(&mut self, mods: i32) {
        self.mods = mods;
    }

    /// The pointer moved to a position inside the framebuffer.
    pub fn moved(&mut self, x: i32, y: i32) {
        self.at = Some((x, y));
    }

    /// The pointer left the framebuffer, releasing whatever it was dragging.
    pub fn left(&mut self) -> Vec<Event> {
        self.at = None;
        let released = std::mem::take(&mut self.held);
        let mods = self.mods;
        released
            .into_iter()
            .map(|(button, (x, y))| at_button("mouseReleased", x, y, button, mods))
            .collect()
    }

    pub fn press(&mut self, x: i32, y: i32, button: Button) -> Event {
        self.at = Some((x, y));
        match self.held.iter_mut().find(|(b, _)| *b == button) {
            Some(slot) => slot.1 = (x, y),
            None => self.held.push((button, (x, y))),
        }
        at_button("mouseClicked", x, y, button, self.mods)
    }

    pub fn release(&mut self, x: i32, y: i32, button: Button) -> Event {
        self.at = Some((x, y));
        self.held.retain(|(b, _)| *b != button);
        at_button("mouseReleased", x, y, button, self.mods)
    }

    /// The per-tick sweep: a drag for each held button that moved, then the move itself.
    pub fn flush(&mut self) -> Vec<Event> {
        let Some(now) = self.at else {
            return Vec::new();
        };
        let mut events = Vec::new();
        for (button, last) in &mut self.held {
            if *last != now {
                *last = now;
                events.push(at_button("mouseDragged", now.0, now.1, *button, self.mods));
            }
        }
        if self.moved != Some(now) {
            self.moved = Some(now);
            events.push(at("mouseMoved", now.0, now.1));
        }
        events
    }
}
