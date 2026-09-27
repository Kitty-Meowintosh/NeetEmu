//! The frames `neetemu exec` and a guest exchange over a `neetemu:port`, specified in `docs/exec.md`.

use crate::port::PortHandle;

/// Host to guest: run this shell statement.
pub const EXEC: u8 = b'X';
/// Host to guest: bytes for the running program's standard input.
pub const STDIN: u8 = b'I';
/// Host to guest: its standard input has ended.
pub const STDIN_EOF: u8 = b'Z';

/// Guest to host: the guest has claimed the port and takes statements.
pub const READY: u8 = b'R';
/// Guest to host: bytes from the running program's standard output.
pub const STDOUT: u8 = b'O';
/// Guest to host: bytes from its standard error.
pub const STDERR: u8 = b'E';
/// Guest to host: the program is finished, and this is its exit status.
pub const STATUS: u8 = b'S';
/// Guest to host: the guest could not do what was asked.
pub const FAULT: u8 = b'!';

/// Largest payload a frame may declare.
pub const MAX_PAYLOAD: usize = 1 << 20;

const HEADER: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub kind: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    /// The payload as text, for the frames that carry a message rather than bytes.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.payload).into_owned()
    }
}

pub fn encode(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + payload.len());
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Reassembles frames from a byte stream that arrives in arbitrary pieces.
#[derive(Default)]
pub struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// The next whole frame, or `Err` once the stream is corrupt.
    pub fn take(&mut self) -> Result<Option<Frame>, String> {
        if self.buffer.len() < HEADER {
            return Ok(None);
        }
        let len = u32::from_be_bytes([
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
            self.buffer[4],
        ]) as usize;
        if len > MAX_PAYLOAD {
            return Err(format!("frame claims {len} bytes"));
        }
        if self.buffer.len() < HEADER + len {
            return Ok(None);
        }
        let kind = self.buffer[0];
        let payload = self.buffer[HEADER..HEADER + len].to_vec();
        self.buffer.drain(..HEADER + len);
        Ok(Some(Frame { kind, payload }))
    }
}

/// What one `exec` produced.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub status: i64,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// The host's side of one port, framed.
pub struct Session {
    port: PortHandle,
    decoder: Decoder,
    /// Bytes the port had no room for last time, sent before anything new.
    backlog: Vec<u8>,
}

impl Session {
    pub fn new(port: PortHandle) -> Session {
        Session {
            port,
            decoder: Decoder::new(),
            backlog: Vec::new(),
        }
    }

    /// Queues a frame, keeping what the port had no room for.
    pub fn send(&mut self, kind: u8, payload: &[u8]) {
        self.backlog.extend_from_slice(&encode(kind, payload));
        self.flush();
    }

    /// Pushes as much of the backlog as the port will take.
    pub fn flush(&mut self) {
        if self.backlog.is_empty() {
            return;
        }
        let sent = self.port.send(&self.backlog);
        self.backlog.drain(..sent);
    }

    /// Every whole frame the guest has written since the last call.
    pub fn poll(&mut self) -> Result<Vec<Frame>, String> {
        self.flush();
        self.decoder.push(&self.port.recv());
        let mut frames = Vec::new();
        while let Some(frame) = self.decoder.take()? {
            frames.push(frame);
        }
        Ok(frames)
    }

    /// Drops what was in flight, for a machine that is starting again.
    pub fn reset(&mut self) {
        self.decoder = Decoder::new();
        self.backlog.clear();
    }

    pub fn close(&self) {
        self.port.close();
    }
}
