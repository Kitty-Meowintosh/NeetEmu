//! `neetemu:port`, a byte channel the guest sees as an ordinary peripheral.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::events::Value;
use crate::peripheral::{Ctx, Peripheral};

/// Unread bytes one direction holds before it stops accepting more.
pub const CAPACITY: usize = 64 * 1024;

/// Largest `read(n)` a guest can ask for in one call.
pub const MAX_READ: usize = 16 * 1024;

#[derive(Default)]
struct Channel {
    to_guest: VecDeque<u8>,
    to_host: VecDeque<u8>,
    host_open: bool,
    guest_open: bool,
    /// Whether the next tick may ring the doorbell; a guest `read` re-arms it.
    armed: bool,
    /// Set when the host end goes away, until the guest is told.
    announce_close: bool,
}

impl Channel {
    fn new() -> Channel {
        Channel {
            host_open: true,
            guest_open: true,
            armed: true,
            ..Channel::default()
        }
    }

    /// Appends as much of `data` as there is room for, reporting how much went in.
    fn push(queue: &mut VecDeque<u8>, data: &[u8]) -> usize {
        let room = CAPACITY.saturating_sub(queue.len());
        let taken = room.min(data.len());
        queue.extend(&data[..taken]);
        taken
    }

    fn take(queue: &mut VecDeque<u8>, n: usize) -> Vec<u8> {
        let n = n.min(queue.len());
        queue.drain(..n).collect()
    }
}

/// The host's end of a port, cloneable and usable while the machine runs.
#[derive(Clone)]
pub struct PortHandle {
    channel: Arc<Mutex<Channel>>,
}

impl PortHandle {
    fn lock(&self) -> std::sync::MutexGuard<'_, Channel> {
        self.channel.lock().expect("port channel poisoned")
    }

    /// Queues bytes for the guest, reporting how many fit.
    pub fn send(&self, data: &[u8]) -> usize {
        let mut channel = self.lock();
        Channel::push(&mut channel.to_guest, data)
    }

    /// Queues every byte, blocking until the guest has read enough; never call it from the ticking thread.
    pub fn send_all(&self, data: &[u8]) {
        let mut at = 0;
        while at < data.len() {
            at += self.send(&data[at..]);
            if at < data.len() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }

    /// Everything the guest has written since the last call.
    pub fn recv(&self) -> Vec<u8> {
        let mut channel = self.lock();
        let n = channel.to_host.len();
        Channel::take(&mut channel.to_host, n)
    }

    /// Bytes the guest has not read yet.
    pub fn pending(&self) -> usize {
        self.lock().to_guest.len()
    }

    /// Drops the host's end.
    pub fn close(&self) {
        let mut channel = self.lock();
        if channel.host_open {
            channel.host_open = false;
            channel.announce_close = true;
            channel.armed = true;
        }
    }

    /// False once the guest has called `close`.
    pub fn is_open(&self) -> bool {
        self.lock().guest_open
    }

    /// Empties both directions and opens the guest's end, leaving the host's alone.
    pub fn reset(&self) {
        let mut channel = self.lock();
        channel.to_guest.clear();
        channel.to_host.clear();
        channel.guest_open = true;
        channel.armed = true;
        channel.announce_close = false;
    }

    /// The guest's end of this channel again, for re-attaching across a reboot.
    pub fn port(&self) -> Port {
        Port {
            channel: Arc::clone(&self.channel),
        }
    }
}

/// One byte channel, as the guest sees it.
pub struct Port {
    channel: Arc<Mutex<Channel>>,
}

impl Port {
    pub const TYPE: &'static str = "neetemu:port";

    pub fn new() -> Port {
        Port {
            channel: Arc::new(Mutex::new(Channel::new())),
        }
    }

    /// The host's end of this port.
    pub fn handle(&self) -> PortHandle {
        PortHandle {
            channel: Arc::clone(&self.channel),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Channel> {
        self.channel.lock().expect("port channel poisoned")
    }
}

impl Default for Port {
    fn default() -> Port {
        Port::new()
    }
}

/// A count argument, which must be a whole number.
fn count(args: &[Value], what: &str) -> Result<usize, String> {
    match args.first() {
        Some(Value::Int(n)) if *n >= 0 => Ok(*n as usize),
        Some(Value::Num(n)) if *n >= 0.0 => Ok(*n as usize),
        _ => Err(format!("#1 {what}: number expected")),
    }
}

fn bytes(args: &[Value]) -> Result<Vec<u8>, String> {
    match args.first() {
        Some(Value::Str(s)) => Ok(s.as_bytes().to_vec()),
        Some(Value::Bytes(b)) => Ok(b.clone()),
        _ => Err("#1 data: string expected".into()),
    }
}

impl Peripheral for Port {
    fn type_name(&self) -> &str {
        Port::TYPE
    }

    fn functions(&self) -> &[&'static str] {
        &["read", "write", "poll", "close"]
    }

    fn host_channel(&self) -> Option<PortHandle> {
        Some(self.handle())
    }

    fn call(&mut self, _ctx: &mut Ctx, name: &str, args: &[Value]) -> Result<Vec<Value>, String> {
        let mut channel = self.lock();
        match name {
            "read" => {
                if !channel.guest_open {
                    return Err("port is closed".into());
                }
                let wanted = count(args, "bytes")?.min(MAX_READ);
                let taken = Channel::take(&mut channel.to_guest, wanted);
                // Any read re-arms the doorbell, so bytes left behind ring again.
                channel.armed = true;
                Ok(vec![Value::Bytes(taken)])
            }
            "write" => {
                if !channel.guest_open {
                    return Err("port is closed".into());
                }
                let data = bytes(args)?;
                if !channel.host_open {
                    return Ok(vec![Value::Int(0)]);
                }
                let taken = Channel::push(&mut channel.to_host, &data);
                Ok(vec![Value::Int(taken as i64)])
            }
            "poll" => Ok(vec![
                Value::Int(channel.to_guest.len() as i64),
                Value::Bool(channel.host_open),
            ]),
            "close" => {
                channel.guest_open = false;
                channel.to_guest.clear();
                Ok(vec![])
            }
            _ => Err(format!("no such function {name}")),
        }
    }

    /// Rings once for waiting bytes and once for a host end that has gone.
    fn tick(&mut self, ctx: &mut Ctx) {
        let (ring, closed) = {
            let mut channel = self.lock();
            if !channel.guest_open {
                return;
            }
            let ring = channel.armed && !channel.to_guest.is_empty();
            if ring {
                channel.armed = false;
            }
            (ring, std::mem::take(&mut channel.announce_close))
        };
        if ring {
            ctx.emit("data", []);
        }
        if closed {
            ctx.emit("closed", []);
        }
    }
}
