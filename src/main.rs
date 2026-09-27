//! The `neetemu` command: windowed or headless runs, and `neetemu exec`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use neetemu::disk::Share;
use neetemu::display::{Display, Spec};
use neetemu::events::Label;
use neetemu::harness::{BootSpec, Harness, BOOT_TIMEOUT, EXEC_TIMEOUT};
use neetemu::serial;
use neetemu::vm::Outcome;
use neetemu::world::{MachineSpec, PeripheralSpec, World, WorldSpec};

const USAGE: &str = "\
usage: neetemu [options]
       neetemu exec [options] <statement>

  --disk-root <dir>   directory holding the numbered disks (default: .)
  --disk <n>          add a machine booting that disk; repeatable
                      neither given, inside a disk directory: that disk
  --world <file>      load the machines from a world TOML file
  --batches <n>       tickets granted per tick (default 3750)
  --tps <n>           ticks per second (default 20, as the mod runs)
  --peripheral <t>[=<tag>]  attach a module to the last --disk; repeatable
  --share <N=/path[,ro]>    show the last --disk a host directory; repeatable
  --network <name>    wire the last --disk to a cable segment; repeatable
  --internet          let machines reach the network (off by default)
  --no-preempt        never force a yield from the host hook
  --clipboard         give machines chip.getClipboard and chip.setClipboard
  --headless          emulate the screen but open no window
  --eval <lua>        run this chunk instead of the disk's entrypoint
  --eval-file <path>  run this file instead of the disk's entrypoint
  --ticks <n>         run at most n ticks, then stop
  --version           print the interpreter versions
  --help

exec runs one statement in a guest that speaks the exec protocol (docs/exec.md)
and exits with its status:

  --disk-root <dir>   directory holding the numbered disks (default: .)
  --disk <n>          the disk to boot
                      neither given, inside a disk directory: that disk
  --share <N=/path[,ro]>    show the machine a host directory; repeatable
  --peripheral <t>[=<tag>]  attach a module to the machine; repeatable
  --network <name>    wire the machine to a cable segment; repeatable
  --tps <n>           pace the machine at n ticks per second (unpaced by default)
  --batches <n>       tickets granted per tick (default 3750)
  --no-preempt        never force a yield from the host hook
  --clipboard         give the machine chip.getClipboard and chip.setClipboard
  --timeout <s>       seconds the statement may take (default 60)
  --boot-timeout <s>  seconds the guest has to send READY (default 60)
  --internet          let the machine reach the network (off by default)
";

struct Options {
    spec: WorldSpec,
    headless: bool,
    max_ticks: Option<u64>,
    /// Replaces every machine's entrypoint.
    eval: Option<Vec<u8>>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        let (lua, yslua) = neetemu::interpreter_version();
        println!("neetemu {}", env!("CARGO_PKG_VERSION"));
        println!("{lua}");
        println!("{yslua}");
        return ExitCode::SUCCESS;
    }

    if args.first().is_some_and(|a| a == "exec") {
        return match exec(&args[1..]) {
            Ok(code) => code,
            Err(message) => {
                eprintln!("neetemu: {message}");
                ExitCode::FAILURE
            }
        };
    }

    let options = match parse(&args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("neetemu: {message}");
            eprint!("\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match run(options) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("neetemu: {message}");
            ExitCode::FAILURE
        }
    }
}

/// `neetemu exec` — one statement through the port, with the guest's own status.
fn exec(args: &[String]) -> Result<ExitCode, String> {
    let mut disk_root: Option<PathBuf> = None;
    let mut disk: Option<u32> = None;
    let mut timeout = EXEC_TIMEOUT;
    let mut boot = BOOT_TIMEOUT;
    let mut spec = BootSpec::default();
    let mut statement: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let next = |i: usize, what: &str| -> Result<String, String> {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{what} needs a value"))
        };
        match args[i].as_str() {
            "--disk-root" => {
                disk_root = Some(PathBuf::from(next(i, "--disk-root")?));
                i += 2;
            }
            "--disk" => {
                let raw = next(i, "--disk")?;
                disk = Some(
                    raw.parse()
                        .map_err(|_| format!("--disk {raw:?} is not a number"))?,
                );
                i += 2;
            }
            "--share" => {
                spec.shares.push(Share::parse(&next(i, "--share")?)?);
                i += 2;
            }
            "--peripheral" => {
                spec.peripherals.push(peripheral(&next(i, "--peripheral")?));
                i += 2;
            }
            "--network" => {
                spec.networks.push(next(i, "--network")?);
                i += 2;
            }
            "--tps" => {
                spec.tps = Some(rate(&next(i, "--tps")?)?);
                i += 2;
            }
            "--batches" => {
                spec.batches = Some(count(&next(i, "--batches")?, "--batches")?);
                i += 2;
            }
            "--no-preempt" => {
                spec.no_preempt = Some(true);
                i += 1;
            }
            "--clipboard" => {
                spec.clipboard = Some(true);
                i += 1;
            }
            "--timeout" => {
                timeout = seconds(&next(i, "--timeout")?, "--timeout")?;
                i += 2;
            }
            "--boot-timeout" => {
                boot = seconds(&next(i, "--boot-timeout")?, "--boot-timeout")?;
                i += 2;
            }
            "--internet" => {
                spec.internet = true;
                i += 1;
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other:?}")),
            _ => {
                if statement.is_some() {
                    return Err("exec takes one statement".into());
                }
                statement = Some(args[i].clone());
                i += 1;
            }
        }
    }

    let Some(statement) = statement else {
        return Err("exec needs a statement".into());
    };

    let (disk_root, disk) = match (disk_root, disk) {
        (root, Some(disk)) => (root.unwrap_or_else(|| PathBuf::from(".")), disk),
        (None, None) => disk_here().ok_or("exec needs --disk, or to run from a disk directory")?,
        (Some(_), None) => return Err("exec needs --disk".into()),
    };

    run_exec(&disk_root, disk, &statement, boot, timeout, spec)
}

/// The working directory's parent and number, when it is a disk directory.
fn disk_here() -> Option<(PathBuf, u32)> {
    let here = std::env::current_dir().ok()?;
    if !here.join("build.json").is_file() {
        return None;
    }
    let disk = here.file_name()?.to_str()?.parse().ok()?;
    Some((here.parent()?.to_path_buf(), disk))
}

/// Parses `--tps`, which the mod runs at 20.
fn rate(raw: &str) -> Result<f64, String> {
    let rate: f64 = raw
        .parse()
        .map_err(|_| format!("--tps {raw:?} is not a number"))?;
    if !(rate > 0.0 && rate <= 1000.0) {
        return Err(format!("--tps {raw} is outside 0 to 1000"));
    }
    Ok(rate)
}

fn count(raw: &str, what: &str) -> Result<i64, String> {
    raw.parse()
        .map_err(|_| format!("{what} {raw:?} is not a number"))
}

/// Parses `--peripheral`, as a type or a type and the tag it answers to.
fn peripheral(raw: &str) -> PeripheralSpec {
    let (kind, tag) = match raw.split_once('=') {
        Some((kind, tag)) => (kind.to_string(), Some(tag.to_string())),
        None => (raw.to_string(), None),
    };
    PeripheralSpec {
        kind,
        tag,
        ..Default::default()
    }
}

fn seconds(raw: &str, what: &str) -> Result<Duration, String> {
    let value: f64 = raw
        .parse()
        .map_err(|_| format!("{what} {raw:?} is not a number"))?;
    if value.is_nan() || value <= 0.0 {
        return Err(format!("{what} {raw} must be positive"));
    }
    Ok(Duration::from_secs_f64(value))
}

fn run_exec(
    disk_root: &Path,
    disk: u32,
    statement: &str,
    boot: Duration,
    timeout: Duration,
    spec: BootSpec,
) -> Result<ExitCode, String> {
    let mut harness = Harness::start(disk_root, disk, spec)?;
    harness.wait_ready(boot)?;

    // Standard input is forwarded as it arrives.
    let (tx, rx) = std::sync::mpsc::channel::<Option<Vec<u8>>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut chunk = [0u8; 4096];
        loop {
            match stdin.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(Some(chunk[..n].to_vec())).is_err() {
                        return;
                    }
                }
            }
        }
        let _ = tx.send(None);
    });

    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    let mut sink = |frame: &serial::Frame| {
        let target: &mut dyn Write = match frame.kind {
            serial::STDERR => &mut err,
            _ => &mut out,
        };
        let _ = target.write_all(&frame.payload);
        let _ = target.flush();
    };

    harness.begin(statement);
    let deadline = std::time::Instant::now() + timeout;
    let mut open = true;
    let status = loop {
        while open {
            match rx.try_recv() {
                Ok(Some(chunk)) => harness.send_stdin(&chunk),
                Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    harness.end_stdin();
                    open = false;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }
        match harness.pump(&mut sink)? {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                return Err(format!("{statement:?} did not finish in time"))
            }
            None => harness.step()?,
        }
    };

    harness.close();
    Ok(ExitCode::from(status.clamp(0, 255) as u8))
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut disk_root: Option<PathBuf> = None;
    let mut disks: Vec<u32> = Vec::new();
    let mut world: Option<PathBuf> = None;
    let mut headless = false;
    let mut max_ticks = None;
    let mut batches: Option<i64> = None;
    let mut tps: Option<f64> = None;
    let mut eval: Option<Vec<u8>> = None;
    let mut no_preempt = false;
    let mut clipboard = false;
    let mut internet = false;
    let mut peripherals: Vec<Vec<PeripheralSpec>> = Vec::new();
    let mut shares: Vec<Vec<Share>> = Vec::new();
    let mut networks: Vec<Vec<String>> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let next = |i: usize, what: &str| -> Result<String, String> {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{what} needs a value"))
        };
        match args[i].as_str() {
            "--disk-root" => {
                disk_root = Some(PathBuf::from(next(i, "--disk-root")?));
                i += 2;
            }
            "--disk" => {
                let raw = next(i, "--disk")?;
                disks.push(
                    raw.parse()
                        .map_err(|_| format!("--disk {raw:?} is not a number"))?,
                );
                peripherals.push(Vec::new());
                shares.push(Vec::new());
                networks.push(Vec::new());
                i += 2;
            }
            "--share" => {
                let share = Share::parse(&next(i, "--share")?)?;
                let Some(machine) = shares.last_mut() else {
                    return Err("--share needs a --disk before it".into());
                };
                machine.push(share);
                i += 2;
            }
            "--network" => {
                let name = next(i, "--network")?;
                let Some(machine) = networks.last_mut() else {
                    return Err("--network needs a --disk before it".into());
                };
                machine.push(name);
                i += 2;
            }
            "--peripheral" => {
                let module = peripheral(&next(i, "--peripheral")?);
                let Some(machine) = peripherals.last_mut() else {
                    return Err("--peripheral needs a --disk before it".into());
                };
                machine.push(module);
                i += 2;
            }
            "--world" => {
                world = Some(PathBuf::from(next(i, "--world")?));
                i += 2;
            }
            "--ticks" => {
                let raw = next(i, "--ticks")?;
                max_ticks = Some(
                    raw.parse()
                        .map_err(|_| format!("--ticks {raw:?} is not a number"))?,
                );
                i += 2;
            }
            "--tps" => {
                tps = Some(rate(&next(i, "--tps")?)?);
                i += 2;
            }
            "--batches" => {
                batches = Some(count(&next(i, "--batches")?, "--batches")?);
                i += 2;
            }
            "--eval" => {
                eval = Some(next(i, "--eval")?.into_bytes());
                i += 2;
            }
            "--eval-file" => {
                let path = next(i, "--eval-file")?;
                eval = Some(std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?);
                i += 2;
            }
            "--internet" => {
                internet = true;
                i += 1;
            }
            "--no-preempt" => {
                no_preempt = true;
                i += 1;
            }
            "--clipboard" => {
                clipboard = true;
                i += 1;
            }
            "--headless" => {
                headless = true;
                i += 1;
            }
            other => return Err(format!("unknown option {other:?}")),
        }
    }

    let spec = match (world, disks.is_empty()) {
        (Some(_), false) => return Err("--world and --disk are mutually exclusive".into()),
        (Some(path), true) => {
            if !peripherals.iter().all(Vec::is_empty) {
                return Err("--peripheral belongs in the world file".into());
            }
            if !shares.iter().all(Vec::is_empty) {
                return Err("--share belongs in the world file".into());
            }
            if !networks.iter().all(Vec::is_empty) {
                return Err("--network belongs in the world file".into());
            }
            if disk_root.is_some() {
                return Err("--disk-root belongs in the world file".into());
            }
            let mut spec = WorldSpec::from_toml(&path)?;
            spec.internet = spec.internet || internet;
            if clipboard {
                for machine in &mut spec.machines {
                    machine.clipboard = Some(true);
                }
            }
            // The flag wins over the world file.
            if tps.is_some() {
                spec.tps = tps;
            }
            spec
        }
        (None, false) => WorldSpec {
            disk_root: disk_root.unwrap_or_else(|| PathBuf::from(".")),
            tps,
            internet,
            machines: disks
                .into_iter()
                .zip(peripherals)
                .zip(shares)
                .zip(networks)
                .map(|(((disk, peripheral), share), networks)| MachineSpec {
                    disk,
                    screen: None,
                    batches,
                    no_preempt: Some(no_preempt),
                    clipboard: Some(clipboard),
                    peripheral,
                    share,
                    networks,
                })
                .collect(),
        },
        (None, true) => match disk_here() {
            Some((root, disk)) => WorldSpec {
                disk_root: root,
                tps,
                internet,
                machines: vec![MachineSpec {
                    disk,
                    screen: None,
                    batches,
                    no_preempt: Some(no_preempt),
                    clipboard: Some(clipboard),
                    peripheral: peripherals.into_iter().next().unwrap_or_default(),
                    share: shares.into_iter().next().unwrap_or_default(),
                    networks: networks.into_iter().next().unwrap_or_default(),
                }],
            },
            None => {
                return Err(
                    "nothing to run: pass --disk or --world, or run from a disk directory".into(),
                )
            }
        },
    };

    Ok(Options {
        spec,
        headless,
        max_ticks,
        eval,
    })
}

/// One window per machine, at that machine's own screen size.
fn windows(spec: &WorldSpec) -> Vec<Spec> {
    spec.machines
        .iter()
        .map(|machine| {
            let config = machine.config();
            Spec {
                title: format!("neetemu \u{2014} disk {}", machine.disk),
                width: config.screen_width,
                height: config.screen_height,
            }
        })
        .collect()
}

fn run(options: Options) -> Result<ExitCode, String> {
    let mut world =
        World::start_with(&options.spec, options.eval.clone()).map_err(|e| e.to_string())?;
    for machine in &world.machines {
        eprintln!("neetemu: machine {} booting {}", machine.id, machine.label);
    }
    let mut display = match options.headless {
        true => None,
        false => match Display::open(&windows(&options.spec)) {
            Ok(display) => Some(display),
            Err(message) => {
                eprintln!("neetemu: {message}, running as if --headless");
                None
            }
        },
    };

    let period = world.period;
    let mut next = std::time::Instant::now();
    while !world.all_stopped() {
        if options.max_ticks.is_some_and(|max| world.ticks >= max) {
            break;
        }
        if let Some(display) = display.as_mut() {
            let input = display.pump();
            for (machine, event) in input.events {
                if let Some(machine) = world.machines.get_mut(machine) {
                    if machine.is_running() {
                        machine.host().queue_event(Label::User, event);
                    }
                }
            }
            for id in input.closed {
                display.close(id);
                if let Some(machine) = world.machines.get_mut(id) {
                    machine.stopped.get_or_insert(Outcome::Shutdown);
                }
            }
            if input.quit || display.is_empty() {
                break;
            }
        }
        world.tick();
        for id in &world.rebooted {
            eprintln!("neetemu: machine {id} rebooting");
        }
        if let Some(display) = display.as_mut() {
            for machine in &mut world.machines {
                if !std::mem::take(&mut machine.host().dirty) {
                    continue;
                }
                let id = machine.id;
                let pixels = machine.host().surfaces.screen().pixels().to_vec();
                display.present(id, &pixels);
            }
        }
        next += period;
        if let Some(wait) = next.checked_duration_since(std::time::Instant::now()) {
            std::thread::sleep(wait);
        } else {
            // Behind schedule: drop the slack.
            next = std::time::Instant::now();
        }
    }

    let mut failed = false;
    for machine in &world.machines {
        match &machine.stopped {
            None => eprintln!("neetemu: machine {} still running", machine.id),
            Some(Outcome::Completed) => eprintln!("neetemu: machine {} halted", machine.id),
            Some(Outcome::Shutdown) => eprintln!("neetemu: machine {} shut down", machine.id),
            Some(Outcome::Crashed(m)) => {
                failed = true;
                eprintln!("neetemu: machine {} crashed: {m}", machine.id);
            }
            Some(Outcome::Error(m)) => {
                failed = true;
                eprintln!("neetemu: machine {} error: {m}", machine.id);
            }
            // Never stored.
            Some(Outcome::Ran) | Some(Outcome::Reboot) => {}
        }
    }
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tick_rate_is_a_number_inside_the_range() {
        assert_eq!(rate("60"), Ok(60.0));
        assert!(rate("0").is_err());
        assert!(rate("1001").is_err());
        assert!(rate("fast").is_err());
    }

    #[test]
    fn a_peripheral_is_a_type_and_an_optional_tag() {
        let plain = peripheral("neetemu:echo");
        assert_eq!(plain.kind, "neetemu:echo");
        assert_eq!(plain.tag, None);

        let tagged = peripheral("neetemu:echo=left");
        assert_eq!(tagged.kind, "neetemu:echo");
        assert_eq!(tagged.tag.as_deref(), Some("left"));
    }
}
