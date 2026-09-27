//! Several machines driven in lockstep, exchanging only messages.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::api;
use crate::disk::{DiskError, Share};
use crate::events::{Event, Label};
use crate::host::{Clock, Host, Outgoing};
use crate::peripheral;
use crate::vm::{Config, Outcome, Vm};

pub struct Machine {
    pub id: usize,
    pub label: String,
    vm: Vm,
    /// `None` while running.
    pub stopped: Option<Outcome>,
    /// How many times `chip.reboot` has rebuilt this machine.
    pub reboots: u32,
    disk_root: PathBuf,
    disk: u32,
    config: Config,
    /// The `--eval` chunk, kept across reboots.
    source: Option<Vec<u8>>,
    /// Re-applied on reboot.
    hardware: Vec<PeripheralSpec>,
    /// Re-applied on reboot, as the hardware is.
    shares: Vec<Share>,
    /// Host ports, re-attached on reboot.
    ports: Vec<crate::port::PortHandle>,
    internet: bool,
    /// Cable segments this machine is on; a machine on none hears nobody.
    pub networks: Vec<String>,
}

#[derive(Debug)]
pub enum StartError {
    Disk(DiskError),
    Vm(String),
    Entrypoint(String),
    Peripheral(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::Disk(e) => write!(f, "{e}"),
            StartError::Vm(e) => write!(f, "{e}"),
            StartError::Entrypoint(e) => write!(f, "{e}"),
            StartError::Peripheral(e) => write!(f, "{e}"),
        }
    }
}

impl Machine {
    pub fn start(
        id: usize,
        disk_root: &Path,
        disk: u32,
        config: Config,
        clock: Clock,
    ) -> Result<Machine, StartError> {
        Machine::start_with(id, disk_root, disk, config, clock, None)
    }

    /// Boots `source` instead of the disk's entrypoint, for `--eval` and the tests.
    pub fn start_with(
        id: usize,
        disk_root: &Path,
        disk: u32,
        config: Config,
        clock: Clock,
        source: Option<Vec<u8>>,
    ) -> Result<Machine, StartError> {
        let vm = Machine::boot(id, disk_root, disk, config, clock, source.as_deref())?;
        Ok(Machine {
            id,
            label: format!("disk {disk}"),
            vm,
            stopped: None,
            reboots: 0,
            disk_root: disk_root.to_path_buf(),
            disk,
            config,
            source,
            hardware: Vec::new(),
            shares: Vec::new(),
            ports: Vec::new(),
            internet: false,
            networks: Vec::new(),
        })
    }

    /// Attaches what is wired to the machine, and keeps it across reboots.
    pub fn install(
        &mut self,
        hardware: Vec<PeripheralSpec>,
        shares: Vec<Share>,
        internet: bool,
    ) -> Result<(), StartError> {
        self.hardware = hardware;
        self.shares = shares;
        self.internet = internet;
        self.wire()
    }

    /// Attaches a host port, kept across reboots.
    pub fn attach_port(&mut self, port: crate::port::Port) -> String {
        let handle = port.handle();
        let uuid = self.host().attach_peripheral(Box::new(port));
        self.ports.push(handle);
        uuid
    }

    fn wire(&mut self) -> Result<(), StartError> {
        let shares = std::mem::take(&mut self.shares);
        for share in &shares {
            self.host()
                .disks
                .attach_share(share)
                .map_err(StartError::Disk)?;
        }
        self.shares = shares;
        let hardware = std::mem::take(&mut self.hardware);
        for attached in &hardware {
            attached.attach(self.host())?;
        }
        self.hardware = hardware;
        for handle in self.ports.clone() {
            handle.reset();
            self.host().attach_peripheral(Box::new(handle.port()));
        }
        if self.internet {
            self.host().internet =
                crate::internet::Internet::with(Box::new(crate::internet::live::Live::new()));
        }
        Ok(())
    }

    /// A fresh interpreter over the same disks, with the entrypoint read again.
    fn boot(
        id: usize,
        disk_root: &Path,
        disk: u32,
        config: Config,
        clock: Clock,
        source: Option<&[u8]>,
    ) -> Result<Vm, StartError> {
        let host = Host::new(id, disk_root, disk, config, clock).map_err(StartError::Disk)?;
        let (entrypoint, language) = {
            let boot = host.disks.boot();
            (boot.entrypoint().to_string(), boot.language().to_string())
        };
        let source = match source {
            Some(source) => source.to_vec(),
            None => {
                let boot = host.disks.boot();
                let resolved = boot.resolve(&entrypoint).ok_or_else(|| {
                    StartError::Entrypoint(format!("bad entrypoint {entrypoint:?}"))
                })?;
                std::fs::read(&resolved.real).map_err(|e| {
                    StartError::Entrypoint(format!("{}: {e}", resolved.real.display()))
                })?
            }
        };

        let mut vm = Vm::new(host, config).map_err(StartError::Vm)?;
        unsafe { api::install(vm.state()) };
        // The chunkname is the `language` field, so tracebacks read `[string "Lua"]`.
        vm.load_entrypoint(&source, &language)
            .map_err(StartError::Entrypoint)?;
        Ok(vm)
    }

    /// `Computer.reboot` — the runtime, screen and event queues go, the disks stay.
    pub fn reboot(&mut self) -> Result<(), StartError> {
        let clock = self.vm.host().clock.restarted();
        self.vm = Machine::boot(
            self.id,
            &self.disk_root,
            self.disk,
            self.config,
            clock,
            self.source.as_deref(),
        )?;
        self.reboots += 1;
        // `maintainState` resets only the internet manager.
        self.wire()
    }

    pub fn host(&mut self) -> &mut Host {
        self.vm.host()
    }

    pub fn is_running(&self) -> bool {
        self.stopped.is_none()
    }

    /// One tick, reporting the outcome; the world's own loop ignores it.
    pub fn tick_once(&mut self) -> Outcome {
        if self.stopped.is_some() {
            return Outcome::Ran;
        }
        let host = self.vm.host();
        if !host.mid_tick {
            host.tick_peripherals();
            host.tick_internet();
            host.check_deadline();
            if host.is_parked() {
                return Outcome::Ran;
            }
        }
        let outcome = self.vm.tick();
        match &outcome {
            Outcome::Ran => {}
            // A reboot is not a stop: the machine comes back on the next tick.
            Outcome::Reboot => {
                if let Err(e) = self.reboot() {
                    self.stopped = Some(Outcome::Error(e.to_string()));
                }
            }
            Outcome::Error(m) => self.stopped = Some(Outcome::Error(m.clone())),
            Outcome::Crashed(m) => self.stopped = Some(Outcome::Crashed(m.clone())),
            Outcome::Completed => self.stopped = Some(Outcome::Completed),
            Outcome::Shutdown => self.stopped = Some(Outcome::Shutdown),
        }
        outcome
    }
}

pub struct World {
    pub machines: Vec<Machine>,
    clock: Clock,
    pub ticks: u64,
    /// Wall-clock time one tick is paced to.
    pub period: std::time::Duration,
    /// Machines that rebooted during the last tick.
    pub rebooted: Vec<usize>,
}

impl World {
    pub fn start(spec: &WorldSpec) -> Result<World, StartError> {
        World::start_with(spec, None)
    }

    /// Boots every machine on `source` instead of its entrypoint.
    pub fn start_with(spec: &WorldSpec, source: Option<Vec<u8>>) -> Result<World, StartError> {
        let clock = Clock::start();
        let period = spec.period();
        let mut machines = Vec::new();
        for (id, machine) in spec.machines.iter().enumerate() {
            let mut config = machine.config();
            config.slice = Some(period);
            let mut started = Machine::start_with(
                id,
                &spec.disk_root,
                machine.disk,
                config,
                clock,
                source.clone(),
            )?;
            started.install(
                machine.peripheral.clone(),
                machine.share.clone(),
                spec.internet,
            )?;
            started.networks = machine.networks.clone();
            machines.push(started);
        }
        Ok(World {
            machines,
            clock,
            ticks: 0,
            period,
            rebooted: Vec::new(),
        })
    }

    pub fn clock(&self) -> Clock {
        self.clock
    }

    /// One 50 ms step: every awake machine runs, then messages are delivered.
    pub fn tick(&mut self) {
        self.rebooted.clear();
        for machine in &mut self.machines {
            if machine.tick_once() == Outcome::Reboot {
                self.rebooted.push(machine.id);
            }
        }
        self.deliver();
        self.ticks += 1;
    }

    /// Moves each machine's outbox out along the cable, and over the air.
    fn deliver(&mut self) {
        let mut posted: Vec<(usize, Outgoing)> = Vec::new();
        for machine in &mut self.machines {
            let from = machine.id;
            for message in machine.host().outbox.drain(..) {
                posted.push((from, message));
            }
        }
        for (from, message) in posted {
            match message {
                Outgoing::Broadcast(args) => self.cable(from, Event::new("networkMessage", args)),
                Outgoing::Wireless(broadcast) => self.wireless(broadcast),
            }
        }
    }

    /// `sendNetworkMessage` walks the cable graph, and a sender is never on its own list.
    fn cable(&mut self, from: usize, event: Event) {
        let segments = match self.machines.get(from) {
            Some(machine) => machine.networks.clone(),
            None => return,
        };
        if segments.is_empty() {
            return;
        }
        for machine in &mut self.machines {
            if machine.id == from || !machine.is_running() {
                continue;
            }
            // `isntDuplicate` means two cables between the same pair still deliver once.
            if machine.networks.iter().any(|n| segments.contains(n)) {
                machine.host().queue_event(Label::Network, event.clone());
            }
        }
    }

    /// Every access point in range hears it, the one that sent it included in the sweep.
    fn wireless(&mut self, broadcast: crate::peripheral::Broadcast) {
        for machine in &mut self.machines {
            if machine.is_running() {
                machine.host().deliver_wireless(&broadcast);
            }
        }
    }

    pub fn all_stopped(&self) -> bool {
        self.machines.iter().all(|m| !m.is_running())
    }

    /// True when no machine has anything to do.
    pub fn all_parked(&mut self) -> bool {
        self.machines
            .iter_mut()
            .all(|m| !m.is_running() || m.host().is_parked())
    }
}

/// A machine's entry in the world.
#[derive(Debug, Deserialize)]
pub struct MachineSpec {
    pub disk: u32,
    /// `[width, height]`, defaulting to the mod's 800x600 colour screen.
    pub screen: Option<[u32; 2]>,
    /// Overrides `batchesPerTick`.
    pub batches: Option<i64>,
    /// Charges tickets but never forces a yield.
    pub no_preempt: Option<bool>,
    /// Gives the guest `chip.getClipboard` and `chip.setClipboard`, off by default.
    pub clipboard: Option<bool>,
    /// Modules attached before boot, which the guest finds through `getPeripherals`.
    #[serde(default)]
    pub peripheral: Vec<PeripheralSpec>,
    /// Host directories this machine sees as partitions on `drive0`.
    #[serde(default, rename = "share")]
    pub share: Vec<Share>,
    /// Cable segments this machine is wired to.
    #[serde(default)]
    pub networks: Vec<String>,
}

/// A module attached to a machine before it boots.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct PeripheralSpec {
    #[serde(rename = "type")]
    pub kind: String,
    pub tag: Option<String>,
    /// Options the module itself takes.
    #[serde(default)]
    pub options: toml::Table,
}

impl PeripheralSpec {
    pub fn attach(&self, host: &mut Host) -> Result<String, StartError> {
        let module =
            peripheral::build(&self.kind, &self.options).map_err(StartError::Peripheral)?;
        let uuid = host.attach_peripheral(module);
        if let Some(tag) = &self.tag {
            host.peripherals.set_tag(&uuid, tag);
        }
        Ok(uuid)
    }
}

impl MachineSpec {
    pub fn config(&self) -> Config {
        let mut config = Config::default();
        if let Some([w, h]) = self.screen {
            config.screen_width = w;
            config.screen_height = h;
        }
        if let Some(batches) = self.batches {
            config.batches_per_tick = batches;
        }
        config.no_preempt = self.no_preempt.unwrap_or(false);
        config.clipboard = self.clipboard.unwrap_or(false);
        config
    }
}

/// The world as `--world` describes it, or as the `--disk` flags build it.
#[derive(Debug, Deserialize)]
pub struct WorldSpec {
    #[serde(default = "default_disk_root", rename = "disk-root")]
    pub disk_root: PathBuf,
    /// Ticks per second for the whole world, where the mod runs at 20.
    #[serde(default)]
    pub tps: Option<f64>,
    /// Lets machines reach the network, as `allow-internet-access` does.
    #[serde(default)]
    pub internet: bool,
    #[serde(default, rename = "machine")]
    pub machines: Vec<MachineSpec>,
}

fn default_disk_root() -> PathBuf {
    PathBuf::from(".")
}

impl WorldSpec {
    /// One tick, as both wall-clock pacing and emulated time.
    pub fn period(&self) -> std::time::Duration {
        match self.tps {
            Some(tps) if tps > 0.0 => std::time::Duration::from_secs_f64(1.0 / tps),
            _ => crate::vm::TICK,
        }
    }

    pub fn from_toml(path: &Path) -> Result<WorldSpec, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut spec: WorldSpec =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        // Paths are relative to the world file.
        if spec.disk_root.is_relative() {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                spec.disk_root = dir.join(&spec.disk_root);
            }
        }
        if spec.machines.is_empty() {
            return Err(format!("{}: no [[machine]] entries", path.display()));
        }
        Ok(spec)
    }
}
