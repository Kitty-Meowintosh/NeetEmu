//! `chip.reboot` and `chip.shutdown`, against `Computer.java`.

mod common;

use common::TestDisk;
use neetemu::host::Clock;
use neetemu::vm::{Config, Outcome};
use neetemu::world::Machine;

/// Counts its boots on disk, leaves state behind on the first, and reports it on the second.
const BOOT: &str = r#"
local function count()
    local ok, handle = pcall(files.open, "system:/boots.txt", "r", 0)
    if not ok or not handle then return 0 end
    local text = handle.read("a")
    handle.close()
    return tonumber(text) or 0
end

local boots = count() + 1
local handle = files.open("system:/boots.txt", "w", 0)
handle.write(tostring(boots))
handle.close()

if boots == 1 then
    screen.fill(0, 0, 9, 9, 1, 2, 3, 255)
    event.queueEvent("User", "leftover", 1)
    chip.reboot()
    return
end

local r, g, b = screen.readPixel(0, 0)
local report = files.open("system:/report.txt", "w", 0)
report.write(string.format("%d %d %d %d", r, g, b, #event.getQueue("User")))
report.close()
chip.shutdown()
"#;

/// Runs the disk's own entrypoint until it stops, or gives up.
fn run(disk: &TestDisk) -> Machine {
    let mut machine =
        Machine::start(0, disk.root(), 1, Config::default(), Clock::start()).expect("start");
    for _ in 0..200 {
        machine.tick_once();
        if machine.stopped.is_some() {
            break;
        }
    }
    machine
}

#[test]
fn a_reboot_restarts_the_machine_instead_of_stopping_it() {
    let disk = TestDisk::new("reboot-restarts");
    std::fs::write(disk.path("system/startup.lua"), BOOT).expect("write");

    let machine = run(&disk);

    assert_eq!(machine.reboots, 1);
    assert_eq!(machine.stopped, Some(Outcome::Shutdown));
    assert_eq!(disk.read("system/boots.txt"), b"2");
}

#[test]
fn a_reboot_clears_the_screen_and_the_event_queues() {
    let disk = TestDisk::new("reboot-clears");
    std::fs::write(disk.path("system/startup.lua"), BOOT).expect("write");

    run(&disk);

    // `maintainState` calls `Graphics.clear()` and `eventManager.reset()` on the way back up.
    assert_eq!(
        String::from_utf8_lossy(&disk.read("system/report.txt")),
        "0 0 0 0"
    );
}

#[test]
fn a_shutdown_stops_the_machine() {
    let disk = TestDisk::new("shutdown-stops");
    std::fs::write(disk.path("system/startup.lua"), b"chip.shutdown()\n").expect("write");

    let machine = run(&disk);

    assert_eq!(machine.stopped, Some(Outcome::Shutdown));
    assert_eq!(machine.reboots, 0);
}

#[test]
fn hardware_survives_a_reboot() {
    // A reboot keeps the peripherals.
    use neetemu::peripheral::Echo;
    use neetemu::world::PeripheralSpec;

    let disk = TestDisk::new("reboot-hardware");
    std::fs::write(disk.path("system/startup.lua"), BOOT).expect("write");

    let mut machine =
        Machine::start(0, disk.root(), 1, Config::default(), Clock::start()).expect("start");
    machine
        .install(
            vec![PeripheralSpec {
                kind: Echo::TYPE.to_string(),
                tag: Some("wired".into()),
                ..Default::default()
            }],
            Vec::new(),
            false,
        )
        .expect("install");

    for _ in 0..200 {
        machine.tick_once();
        if machine.stopped.is_some() {
            break;
        }
    }

    assert_eq!(machine.reboots, 1);
    let ids = machine.host().peripherals.ids();
    assert_eq!(ids.len(), 1, "the module should still be attached");
    assert_eq!(machine.host().peripherals.tag_of(&ids[0]), Some("wired"));
}
