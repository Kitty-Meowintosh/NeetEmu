//! The six labelled event queues, ported from `simulation/events/EventManager.java` and `EventQueue.java`.

use std::collections::VecDeque;

/// `simulation/events/EventLabel.java`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    Unlabeled,
    User,
    System,
    Network,
    Peripheral,
    Compatibility,
}

pub const LABELS: [Label; 6] = [
    Label::Unlabeled,
    Label::User,
    Label::System,
    Label::Network,
    Label::Peripheral,
    Label::Compatibility,
];

impl Label {
    /// Case-insensitive, as in `EventAPI.decodeEventLabel`.
    pub fn parse(name: &str) -> Option<Label> {
        LABELS
            .into_iter()
            .find(|l| l.name().eq_ignore_ascii_case(name))
    }

    pub fn name(self) -> &'static str {
        match self {
            Label::Unlabeled => "UNLABELED",
            Label::User => "USER",
            Label::System => "SYSTEM",
            Label::Network => "NETWORK",
            Label::Peripheral => "PERIPHERAL",
            Label::Compatibility => "COMPATIBILITY",
        }
    }

    fn index(self) -> usize {
        LABELS.iter().position(|l| *l == self).unwrap()
    }
}

/// A host callable handed to the guest inside an event payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostFn {
    WebsocketSend(i32),
    WebsocketClose(i32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    /// Bytes as they are, for a body that is not text.
    Bytes(Vec<u8>),
    /// A string-keyed table, as `InternetManager.fromHeaders` builds.
    Map(Vec<(String, String)>),
    Fn(HostFn),
}

#[derive(Clone, Debug)]
pub struct Event {
    pub name: String,
    pub args: Vec<Value>,
}

impl Event {
    pub fn new(name: impl Into<String>, args: Vec<Value>) -> Event {
        Event {
            name: name.into(),
            args,
        }
    }
}

/// `EventManager.maxQueueSize`.
const MAX_QUEUE_SIZE: usize = 75;

#[derive(Default)]
pub struct EventManager {
    queues: [VecDeque<Event>; 6],
}

impl EventManager {
    pub fn new() -> EventManager {
        EventManager::default()
    }

    /// Appends, dropping the oldest once the queue passes 75.
    pub fn queue(&mut self, label: Label, event: Event) {
        let q = &mut self.queues[label.index()];
        q.push_back(event);
        if q.len() > MAX_QUEUE_SIZE {
            q.pop_front();
        }
    }

    /// `getQueue(label)` — reads and clears.
    pub fn take(&mut self, label: Label) -> Vec<Event> {
        self.queues[label.index()].drain(..).collect()
    }

    /// `readQueue(label)` — reads without clearing.
    pub fn read(&self, label: Label) -> Vec<Event> {
        self.queues[label.index()].iter().cloned().collect()
    }

    /// `getQueue(label, filter)`, taking every event of that name.
    pub fn take_filtered(&mut self, label: Label, filter: &str) -> Vec<Event> {
        let q = &mut self.queues[label.index()];
        let mut taken = Vec::new();
        let mut kept = VecDeque::with_capacity(q.len());
        for event in q.drain(..) {
            if event.name == filter {
                taken.push(event);
            } else {
                kept.push_back(event);
            }
        }
        *q = kept;
        taken
    }

    /// `readQueue(label, filter)`.
    pub fn read_filtered(&self, label: Label, filter: &str) -> Vec<Event> {
        self.queues[label.index()]
            .iter()
            .filter(|e| e.name == filter)
            .cloned()
            .collect()
    }

    /// `getFirst(label)` — pops the oldest.
    pub fn take_first(&mut self, label: Label) -> Option<Event> {
        self.queues[label.index()].pop_front()
    }

    /// `getFirst(label, filter)` — pops the oldest of that name.
    pub fn take_first_filtered(&mut self, label: Label, filter: &str) -> Option<Event> {
        let q = &mut self.queues[label.index()];
        let at = q.iter().position(|e| e.name == filter)?;
        q.remove(at)
    }

    pub fn clear(&mut self, label: Label) {
        self.queues[label.index()].clear();
    }

    pub fn reset(&mut self) {
        for q in &mut self.queues {
            q.clear();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.queues.iter().all(VecDeque::is_empty)
    }
}
