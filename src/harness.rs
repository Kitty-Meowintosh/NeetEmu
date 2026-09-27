//! Driving one machine over a `neetemu:port`, for `neetemu exec` and the tests.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::disk::Share;
use crate::port::Port;
use crate::serial::{self, Frame, Output, Session};
use crate::vm::Outcome;
use crate::world::{MachineSpec, PeripheralSpec, StartError, World, WorldSpec};

/// How long a boot may take before `wait_ready` gives up.
pub const BOOT_TIMEOUT: Duration = Duration::from_secs(60);

/// How long one statement may run before `exec` gives up.
pub const EXEC_TIMEOUT: Duration = Duration::from_secs(60);

/// Wall-clock nap taken while every machine is parked.
const IDLE: Duration = Duration::from_millis(2);

/// What a boot varies from the defaults.
#[derive(Default)]
pub struct BootSpec {
    /// Host directories the machine sees as partitions on `drive0`.
    pub shares: Vec<Share>,
    pub internet: bool,
    /// Paces the machine at this many ticks per second.
    pub tps: Option<f64>,
    pub batches: Option<i64>,
    pub no_preempt: Option<bool>,
    pub clipboard: Option<bool>,
    pub peripherals: Vec<PeripheralSpec>,
    pub networks: Vec<String>,
}

pub struct Harness {
    world: World,
    session: Session,
    /// When the next tick is due, on a session paced to a tick rate.
    pace: Option<Instant>,
}

impl Harness {
    /// Boots `disk` headless with one port attached, as `boot` describes it.
    pub fn start(disk_root: &Path, disk: u32, boot: BootSpec) -> Result<Harness, String> {
        Harness::start_with(disk_root, disk, None, boot)
    }

    /// Boots `source` in place of the disk's entrypoint.
    pub fn start_with(
        disk_root: &Path,
        disk: u32,
        source: Option<Vec<u8>>,
        boot: BootSpec,
    ) -> Result<Harness, String> {
        let spec = WorldSpec {
            disk_root: PathBuf::from(disk_root),
            tps: boot.tps,
            internet: boot.internet,
            machines: vec![MachineSpec {
                disk,
                screen: None,
                batches: boot.batches,
                no_preempt: boot.no_preempt,
                clipboard: boot.clipboard,
                peripheral: boot.peripherals,
                share: boot.shares,
                networks: boot.networks,
            }],
        };
        let mut world = World::start_with(&spec, source).map_err(|e: StartError| e.to_string())?;
        let port = Port::new();
        let handle = port.handle();
        world.machines[0].attach_port(port);
        let pace = boot.tps.map(|_| Instant::now());
        Ok(Harness {
            world,
            session: Session::new(handle),
            pace,
        })
    }

    pub fn world(&mut self) -> &mut World {
        &mut self.world
    }

    /// What stopped the machine, if anything has.
    pub fn outcome(&self) -> Option<&Outcome> {
        self.world.machines[0].stopped.as_ref()
    }

    /// One tick, paced to the tick rate or napping while the guest has nothing to do.
    pub fn step(&mut self) -> Result<(), String> {
        if let Some(next) = self.pace.as_mut() {
            match next.checked_duration_since(Instant::now()) {
                Some(wait) => std::thread::sleep(wait),
                // Behind schedule: drop the slack.
                None => *next = Instant::now(),
            }
            *next += self.world.period;
        } else if self.world.all_parked() {
            std::thread::sleep(IDLE);
        }
        self.world.tick();
        if !self.world.rebooted.is_empty() {
            self.session.reset();
        }
        match &self.world.machines[0].stopped {
            None => Ok(()),
            Some(Outcome::Crashed(m)) => Err(format!("the machine crashed: {m}")),
            Some(Outcome::Error(m)) => Err(format!("the machine failed: {m}")),
            Some(other) => Err(format!("the machine stopped: {other:?}")),
        }
    }

    /// Ticks until the guest sends `READY`, returning its payload.
    pub fn wait_ready(&mut self, timeout: Duration) -> Result<String, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(frame) = self.session.poll()?.into_iter().next() {
                return match frame.kind {
                    serial::READY => Ok(frame.text()),
                    serial::FAULT => Err(format!("the guest failed: {}", frame.text())),
                    other => Err(format!("unexpected frame {:?} before ready", other as char)),
                };
            }
            if Instant::now() >= deadline {
                return Err("the guest never sent READY".into());
            }
            self.step()?;
        }
    }

    /// Runs one shell statement, collecting everything it wrote.
    pub fn exec(&mut self, statement: &str, timeout: Duration) -> Result<Output, String> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = self.exec_with(statement, timeout, &mut |frame| match frame.kind {
            serial::STDOUT => stdout.extend_from_slice(&frame.payload),
            serial::STDERR => stderr.extend_from_slice(&frame.payload),
            _ => {}
        })?;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }

    /// Runs one shell statement, handing each output frame to `sink` as it arrives.
    pub fn exec_with(
        &mut self,
        statement: &str,
        timeout: Duration,
        sink: &mut dyn FnMut(&Frame),
    ) -> Result<i64, String> {
        self.begin(statement);
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.pump(sink)? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(format!("{statement:?} did not finish in time"));
            }
            self.step()?;
        }
    }

    /// Asks the guest to run a statement, without waiting for it.
    pub fn begin(&mut self, statement: &str) {
        self.session.send(serial::EXEC, statement.as_bytes());
    }

    /// Reads what the guest has written, `Some(status)` once it finishes; the caller ticks with `step`.
    pub fn pump(&mut self, sink: &mut dyn FnMut(&Frame)) -> Result<Option<i64>, String> {
        for frame in self.session.poll()? {
            match frame.kind {
                serial::STATUS => {
                    let text = frame.text();
                    return text
                        .trim()
                        .parse()
                        .map(Some)
                        .map_err(|_| format!("bad exit status {text:?}"));
                }
                serial::FAULT => return Err(format!("the guest failed: {}", frame.text())),
                serial::READY => {}
                _ => sink(&frame),
            }
        }
        Ok(None)
    }

    /// Queues bytes for the running program's standard input.
    pub fn send_stdin(&mut self, data: &[u8]) {
        self.session.send(serial::STDIN, data);
    }

    /// States that no more standard input is coming.
    pub fn end_stdin(&mut self) {
        self.session.send(serial::STDIN_EOF, &[]);
    }

    /// Drops the host's end, which tells the guest to let go of the port.
    pub fn close(&mut self) {
        self.session.close();
    }
}
