//! Per-machine state the shims reach through the thread's extra space.

use std::path::Path;
use std::time::Instant;

use crate::disk::{DiskError, DiskSet};
use crate::events::{Event, EventManager, Label, Value};
use crate::filehandle::FileHandle;
use crate::internet::Internet;
use crate::peripheral::{Peripheral, Peripherals};
use crate::screen::Surfaces;
use crate::vm::{Config, Outcome};

/// The host's own clock, which `ChipAPI` reads too.
#[derive(Clone, Copy)]
pub struct Clock {
    origin: Instant,
    /// Seconds since the Unix epoch at `origin`.
    unix_origin: f64,
}

impl Clock {
    pub fn start() -> Clock {
        Clock {
            origin: Instant::now(),
            unix_origin: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0),
        }
    }

    /// The same Unix time, but zero uptime, for a machine that just rebooted.
    pub fn restarted(&self) -> Clock {
        Clock {
            origin: Instant::now(),
            unix_origin: self.unix(),
        }
    }

    /// Seconds since this machine was switched on.
    pub fn uptime(&self) -> f64 {
        self.origin.elapsed().as_secs_f64()
    }

    /// `chip.getUnixTime`, at millisecond resolution or better.
    pub fn unix(&self) -> f64 {
        self.unix_origin + self.uptime()
    }
}

/// Why a parked machine was woken.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Woke {
    Event,
    Timeout,
}

impl Woke {
    pub fn name(self) -> &'static str {
        match self {
            Woke::Event => "event",
            Woke::Timeout => "timeout",
        }
    }
}

/// A message a machine wants delivered to its peers.
pub enum Outgoing {
    /// `io.broadcastLocal`.
    Broadcast(Vec<Value>),
    /// An access point's `broadcast`, which only the world can place in range.
    Wireless(crate::peripheral::Broadcast),
}

pub struct Host {
    pub id: usize,
    pub config: Config,
    pub clock: Clock,

    /// Hook intervals still owed to the script; carries over between ticks.
    pub tickets: i64,
    yield_requested: bool,
    exit: Option<Outcome>,

    pub disks: DiskSet,
    pub handles: Vec<Option<FileHandle>>,
    pub events: EventManager,
    pub peripherals: Peripherals,
    pub internet: Internet,
    pub surfaces: Surfaces,
    pub outbox: Vec<Outgoing>,

    /// True when the last tick stopped part-way through its ticket allowance.
    pub mid_tick: bool,
    /// When the running resume must hand control back, if a slice is in force.
    slice_deadline: Option<Instant>,

    /// Set by `screen.draw`; the display presents and clears it.
    pub dirty: bool,

    /// `Some(deadline)` parks until that emulated time, `Some(None)` until an event.
    park: Option<Option<f64>>,
    woke: Woke,
}

impl Host {
    pub fn new(
        id: usize,
        disk_root: &Path,
        disk: u32,
        config: Config,
        clock: Clock,
    ) -> Result<Host, DiskError> {
        Ok(Host {
            id,
            config,
            clock,
            tickets: 0,
            yield_requested: false,
            exit: None,
            disks: DiskSet::load(disk_root, disk)?,
            handles: Vec::new(),
            events: EventManager::new(),
            peripherals: Peripherals::new(),
            internet: Internet::offline(),
            surfaces: Surfaces::new(config.screen_width, config.screen_height),
            outbox: Vec::new(),
            mid_tick: false,
            slice_deadline: None,
            dirty: false,
            park: None,
            woke: Woke::Timeout,
        })
    }

    /// A machine with no disks, for tests that exercise the interpreter alone.
    pub fn bare(config: Config, clock: Clock) -> Host {
        Host {
            id: 0,
            config,
            clock,
            tickets: 0,
            yield_requested: false,
            exit: None,
            disks: DiskSet::empty(),
            handles: Vec::new(),
            events: EventManager::new(),
            peripherals: Peripherals::new(),
            internet: Internet::offline(),
            surfaces: Surfaces::new(config.screen_width, config.screen_height),
            outbox: Vec::new(),
            mid_tick: false,
            slice_deadline: None,
            dirty: false,
            park: None,
            woke: Woke::Timeout,
        }
    }

    /// Seconds since this machine was switched on.
    pub fn uptime(&self) -> f64 {
        self.clock.uptime()
    }

    /// `chip.getUnixTime`.
    pub fn unix(&self) -> f64 {
        self.clock.unix()
    }

    /// Starts a slice, after which the host takes a turn even with tickets left.
    pub fn start_slice(&mut self, slice: Option<std::time::Duration>) {
        self.slice_deadline = slice.map(|d| Instant::now() + d);
    }

    pub fn slice_expired(&self) -> bool {
        self.slice_deadline.is_some_and(|end| Instant::now() >= end)
    }

    pub fn request_yield(&mut self) {
        self.yield_requested = true;
    }

    pub fn take_yield_request(&mut self) -> bool {
        std::mem::replace(&mut self.yield_requested, false)
    }

    /// Records the first kill reason; later ones are ignored.
    pub fn set_exit(&mut self, outcome: Outcome) {
        if self.exit.is_none() {
            self.exit = Some(outcome);
        }
    }

    pub fn take_exit(&mut self) -> Option<Outcome> {
        self.exit.take()
    }

    /// Stores a handle and returns the slot its closures will carry.
    pub fn add_handle(&mut self, handle: FileHandle) -> i64 {
        if let Some(slot) = self.handles.iter().position(Option::is_none) {
            self.handles[slot] = Some(handle);
            return slot as i64;
        }
        self.handles.push(Some(handle));
        self.handles.len() as i64 - 1
    }

    pub fn handle_mut(&mut self, slot: i64) -> Option<&mut FileHandle> {
        self.handles.get_mut(usize::try_from(slot).ok()?)?.as_mut()
    }

    /// Queues an event, waking the machine if it was parked.
    pub fn queue_event(&mut self, label: Label, event: Event) {
        self.events.queue(label, event);
        self.wake(Woke::Event);
    }

    /// Attaches a module present at boot, without raising an event.
    pub fn attach_peripheral(&mut self, module: Box<dyn Peripheral>) -> String {
        self.peripherals.attach(self.id, module)
    }

    /// Attaches a module to a running machine, raising `peripheralAttached`.
    pub fn plug_peripheral(&mut self, module: Box<dyn Peripheral>) -> String {
        let uuid = self.peripherals.attach(self.id, module);
        if let Some((label, event)) = self.peripherals.attached_event(&uuid) {
            self.queue_event(label, event);
        }
        uuid
    }

    /// Detaches a module, raising `peripheralDetached`.
    pub fn unplug_peripheral(&mut self, uuid: &str) -> bool {
        match self.peripherals.detach(uuid) {
            Some((label, event)) => {
                self.queue_event(label, event);
                true
            }
            None => false,
        }
    }

    /// `None` when no peripheral carries that uuid or that function name.
    pub fn call_peripheral(
        &mut self,
        uuid: &str,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Vec<Value>, String>> {
        let (result, pending) = self.peripherals.call(uuid, name, args)?;
        self.raise_all(pending);
        Some(result)
    }

    /// Gives every module its turn, once per tick.
    pub fn tick_peripherals(&mut self) {
        let pending = self.peripherals.tick();
        self.raise_all(pending);
    }

    /// Delivers whatever the network finished since the last tick.
    pub fn tick_internet(&mut self) {
        for event in self.internet.drain() {
            self.queue_event(Label::Network, event);
        }
    }

    /// Offers a broadcast to every module on this machine.
    pub fn deliver_wireless(&mut self, broadcast: &crate::peripheral::Broadcast) {
        let raised = self
            .peripherals
            .receive(&crate::peripheral::Incoming::Wireless(broadcast));
        self.raise_all(raised);
    }

    fn raise_all(&mut self, raised: crate::peripheral::Raised) {
        if crate::peripheral::drain(&mut self.events, raised.events) {
            self.wake(Woke::Event);
        }
        self.outbox.extend(raised.posts);
    }

    /// Stops the machine being resumed until `until`, or until an event arrives.
    pub fn park_until(&mut self, until: Option<f64>) {
        self.park = Some(until);
    }

    pub fn is_parked(&self) -> bool {
        self.park.is_some()
    }

    fn wake(&mut self, woke: Woke) {
        if self.park.is_some() {
            self.park = None;
            self.woke = woke;
        }
    }

    /// Wakes the machine if its deadline has passed; call once per tick.
    pub fn check_deadline(&mut self) {
        if let Some(Some(deadline)) = self.park {
            if self.uptime() >= deadline {
                self.wake(Woke::Timeout);
            }
        }
    }

    /// Why the last park ended.
    pub fn woke(&self) -> Woke {
        self.woke
    }

    /// The emulated time the machine next needs a tick, if it is parked with a deadline.
    pub fn wake_at(&self) -> Option<f64> {
        match self.park {
            Some(Some(deadline)) => Some(deadline),
            _ => None,
        }
    }
}
