//! `internet` as a host module, ported from `simulation/InternetManager.java`.

pub mod live;

use crate::events::{Event, HostFn, Value};

/// `internet-max-sockets`, which the mod reads from the server config.
pub const MAX_SOCKETS: usize = 4;

/// `InternetManager.errorCodes`, which every other status falls through.
fn status_message(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "BAD REQUEST",
        401 => "UNAUTHORIZED",
        403 => "FORBIDDEN",
        404 => "NOT FOUND",
        408 => "TIMEOUT",
        418 => "I'M A TEAPOT",
        429 => "TOO MANY REQUESTS",
        502 => "BAD GATEWAY",
        505 => "HTTP VERSION NOT SUPPORTED",
        _ => "UNKNOWN STATUS CODE",
    }
}

/// `queueResponse`, which has no body slot at all when the body is empty.
pub fn response(
    id: i32,
    code: i32,
    message: &str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> Event {
    let mut args = vec![
        Value::Int(id as i64),
        Value::Int(code as i64),
        Value::Str(message.into()),
        Value::Map(headers),
    ];
    if !body.is_empty() {
        args.push(Value::Bytes(body));
    }
    Event::new("HttpResponse", args)
}

/// A response carrying a status the mod knows a name for.
pub fn status_response(id: i32, code: u16, headers: Vec<(String, String)>, body: Vec<u8>) -> Event {
    response(id, code as i32, status_message(code), headers, body)
}

/// `WebsocketOpened`, whose payload hands the guest two callables.
pub fn opened(id: i32) -> Event {
    Event::new(
        "WebsocketOpened",
        vec![
            Value::Int(id as i64),
            Value::Fn(HostFn::WebsocketSend(id)),
            Value::Fn(HostFn::WebsocketClose(id)),
        ],
    )
}

pub fn message(id: i32, body: Vec<u8>, binary: bool) -> Event {
    Event::new(
        "WebsocketMessage",
        vec![
            Value::Int(id as i64),
            Value::Bytes(body),
            Value::Bool(binary),
        ],
    )
}

pub fn closed(id: i32, code: i32, reason: &str) -> Event {
    Event::new(
        "WebsocketClosed",
        vec![
            Value::Int(id as i64),
            Value::Int(code as i64),
            Value::Str(reason.into()),
        ],
    )
}

pub fn send_failure(id: i32, reason: &str) -> Event {
    Event::new(
        "WebsocketSendFailure",
        vec![Value::Int(id as i64), Value::Str(reason.into())],
    )
}

pub fn send_success(id: i32) -> Event {
    Event::new("WebsocketSendSuccess", vec![Value::Int(id as i64)])
}

/// The transport behind `internet`.
pub trait Backend: Send {
    fn get(&mut self, id: i32, url: &str, headers: Vec<(String, String)>);

    fn post(&mut self, id: i32, url: &str, headers: Vec<(String, String)>, body: Vec<u8>);

    /// `Err` reaches the guest as an `ExposedError`.
    fn open_socket(
        &mut self,
        id: i32,
        url: &str,
        headers: Vec<(String, String)>,
    ) -> Result<(), String>;

    fn send(&mut self, id: i32, data: Vec<u8>, binary: bool);

    fn close(&mut self, id: i32) -> Result<(), String>;

    /// Everything that completed since the last call.
    fn drain(&mut self) -> Vec<Event>;

    /// Drops every live socket, as `InternetManager.reset` does on a reboot.
    fn reset(&mut self) {}

    /// Sockets the guest still holds, against `MAX_SOCKETS`.
    fn live_sockets(&self) -> usize {
        0
    }
}

/// The `internet` table's own state, whichever backend is behind it.
pub struct Internet {
    backend: Option<Box<dyn Backend>>,
    pending: Vec<Event>,
    next_id: i32,
}

impl Internet {
    /// A server with internet access switched off.
    pub fn offline() -> Internet {
        Internet {
            backend: None,
            pending: Vec::new(),
            next_id: 1,
        }
    }

    pub fn with(backend: Box<dyn Backend>) -> Internet {
        Internet {
            backend: Some(backend),
            pending: Vec::new(),
            next_id: 1,
        }
    }

    /// `InternetManager.hasAccess`.
    pub fn has_access(&self) -> bool {
        self.backend.is_some()
    }

    /// `InternetManager.ready`, which is false once the outgoing buffer is full.
    pub fn ready(&self) -> bool {
        self.has_access()
    }

    /// `generateID`, counted rather than random.
    fn next(&mut self) -> i32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    pub fn get(&mut self, url: &str, headers: Vec<(String, String)>) -> i32 {
        let id = self.next();
        match &mut self.backend {
            None => self
                .pending
                .push(response(id, 111, "ECONNREFUSED", Vec::new(), Vec::new())),
            Some(backend) => backend.get(id, url, headers),
        }
        id
    }

    pub fn post(&mut self, url: &str, headers: Vec<(String, String)>, body: Vec<u8>) -> i32 {
        let id = self.next();
        match &mut self.backend {
            None => self
                .pending
                .push(response(id, 111, "ECONNREFUSED", Vec::new(), Vec::new())),
            Some(backend) => backend.post(id, url, headers, body),
        }
        id
    }

    /// Raises on failure instead of queueing it.
    pub fn open_socket(
        &mut self,
        url: &str,
        headers: Vec<(String, String)>,
    ) -> Result<i32, String> {
        let id = self.next();
        let Some(backend) = &mut self.backend else {
            return Err("Internet access disabled".into());
        };
        if backend.live_sockets() >= MAX_SOCKETS {
            return Err("Limit on active sockets reached".into());
        }
        backend.open_socket(id, url, headers)?;
        Ok(id)
    }

    pub fn send(&mut self, id: i32, data: Vec<u8>, binary: bool) {
        match &mut self.backend {
            None => self.pending.push(send_failure(id, "Line closed")),
            Some(backend) => backend.send(id, data, binary),
        }
    }

    pub fn close(&mut self, id: i32) -> Result<(), String> {
        match &mut self.backend {
            None => Err("Already closed".into()),
            Some(backend) => backend.close(id),
        }
    }

    /// Completions to queue on the `Network` label, oldest first.
    pub fn drain(&mut self) -> Vec<Event> {
        let mut events = std::mem::take(&mut self.pending);
        if let Some(backend) = &mut self.backend {
            events.extend(backend.drain());
        }
        events
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        if let Some(backend) = &mut self.backend {
            backend.reset();
        }
    }
}

impl Default for Internet {
    fn default() -> Internet {
        Internet::offline()
    }
}
