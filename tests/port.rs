//! `neetemu:port`, the emulator's own byte channel.

mod common;

use std::time::Duration;

use common::{expect_clean, TestDisk};
use neetemu::events::Label;
use neetemu::host::Clock;
use neetemu::port::{Port, PortHandle, CAPACITY};
use neetemu::vm::{Config, Outcome};
use neetemu::world::Machine;

/// Runs `body` against a machine with one port, handing the test the host's end.
fn with_port(name: &str, body: &str, before: impl FnOnce(&PortHandle)) -> PortHandle {
    let disk = TestDisk::new(name);
    let port = Port::new();
    let handle = port.handle();
    before(&handle);
    expect_clean(disk.run_setup(Config::default(), body, move |host| {
        host.attach_peripheral(Box::new(port));
    }));
    handle
}

/// A machine the test ticks itself, for anything that has to happen mid-run.
fn interactive(name: &str, body: &str) -> (Machine, PortHandle) {
    let disk = TestDisk::new(name);
    let mut machine = Machine::start_with(
        0,
        disk.root(),
        1,
        Config::default(),
        Clock::start(),
        Some(body.as_bytes().to_vec()),
    )
    .expect("boot");
    let port = Port::new();
    let handle = port.handle();
    machine.host().attach_peripheral(Box::new(port));
    (machine, handle)
}

#[test]
fn a_port_is_listed_as_a_peripheral() {
    with_port(
        "port-listed",
        r#"
        local id = io.getPeripherals()[1]
        same("its type", io.getType(id), "neetemu:port")
        local p = io.wrapPeripheral(id)
        for _, name in ipairs({ "read", "write", "poll", "close" }) do
            check(name .. " is on the wrapper", type(p[name]) == "function")
        end
    "#,
        |_| {},
    );
}

#[test]
fn the_guest_reads_what_the_host_sent() {
    with_port(
        "port-read",
        r#"
        local p = io.wrapPeripheral(io.getPeripherals()[1])
        same("the whole message", p.read(64), "from the host")
        same("nothing left", p.read(64), "")
    "#,
        |host| {
            assert_eq!(host.send(b"from the host"), 13);
        },
    );
}

#[test]
fn a_read_is_capped_at_what_was_asked_for() {
    with_port(
        "port-read-capped",
        r#"
        local p = io.wrapPeripheral(io.getPeripherals()[1])
        same("first four", p.read(4), "abcd")
        same("the rest", p.read(64), "efgh")
    "#,
        |host| {
            host.send(b"abcdefgh");
        },
    );
}

#[test]
fn the_host_reads_what_the_guest_wrote() {
    let host = with_port(
        "port-write",
        r#"
        local p = io.wrapPeripheral(io.getPeripherals()[1])
        same("every byte was taken", p.write("to the host"), 11)
    "#,
        |_| {},
    );
    assert_eq!(host.recv(), b"to the host");
    assert!(host.recv().is_empty(), "a second read should find nothing");
}

#[test]
fn binary_bytes_survive_the_round_trip() {
    // A payload that is not text comes back unchanged.
    let host = with_port(
        "port-binary",
        r#"
        local p = io.wrapPeripheral(io.getPeripherals()[1])
        local got = p.read(8)
        same("what the host sent", got, "\255\254\0\1z")
        p.write(got)
    "#,
        |host| {
            host.send(&[0xFF, 0xFE, 0x00, 0x01, b'z']);
        },
    );
    assert_eq!(host.recv(), vec![0xFF, 0xFE, 0x00, 0x01, b'z']);
}

#[test]
fn poll_reports_what_is_waiting_and_whether_the_host_is_there() {
    with_port(
        "port-poll",
        r#"
        local p = io.wrapPeripheral(io.getPeripherals()[1])
        local waiting, open = p.poll()
        same("five bytes waiting", waiting, 5)
        same("the host is still there", open, true)
        p.read(5)
        same("nothing waiting now", (p.poll()), 0)
    "#,
        |host| {
            host.send(b"hello");
        },
    );
}

#[test]
fn a_closed_host_end_shows_up_in_poll() {
    with_port(
        "port-host-closed",
        r#"
        local p = io.wrapPeripheral(io.getPeripherals()[1])
        local waiting, open = p.poll()
        same("the last bytes are still readable", waiting, 3)
        same("but the host has gone", open, false)
        same("and a write is refused", p.write("nobody there"), 0)
    "#,
        |host| {
            host.send(b"bye");
            host.close();
        },
    );
}

#[test]
fn a_write_past_capacity_is_short() {
    let host = with_port(
        "port-backpressure",
        &format!(
            r#"
        local p = io.wrapPeripheral(io.getPeripherals()[1])
        local block = string.rep("x", {capacity})
        same("the first write fills it", p.write(block), {capacity})
        same("the next one is refused", p.write("more"), 0)
    "#,
            capacity = CAPACITY
        ),
        |_| {},
    );
    assert_eq!(host.recv().len(), CAPACITY);
}

#[test]
fn close_drops_the_guests_end() {
    let host = with_port(
        "port-guest-closed",
        r#"
        local p = io.wrapPeripheral(io.getPeripherals()[1])
        p.close()
        raises("read after close", "port is closed", p.read, 1)
        raises("write after close", "port is closed", p.write, "x")
    "#,
        |_| {},
    );
    assert!(!host.is_open(), "the host should see the guest's end go");
}

#[test]
fn the_doorbell_wakes_a_parked_machine() {
    // An attached but idle port costs nothing.
    let (mut machine, host) = interactive(
        "port-doorbell",
        r#"
        local p = io.wrapPeripheral(io.getPeripherals()[1])
        local woke = chip.sleep()
        local got = p.read(64)
        p.write(woke .. ":" .. got)
    "#,
    );

    for _ in 0..20 {
        if machine.host().is_parked() {
            break;
        }
        machine.tick_once();
    }
    assert!(
        machine.host().is_parked(),
        "the guest should be parked on chip.sleep"
    );

    host.send(b"wake up");
    for _ in 0..20 {
        if !machine.is_running() {
            break;
        }
        machine.tick_once();
    }
    let raised = machine.host().events.read(Label::Peripheral);
    assert_eq!(raised.len(), 1, "the doorbell should have rung once");
    assert_eq!(host.recv(), b"event:wake up");
}

#[test]
fn the_doorbell_is_one_event_until_the_guest_reads() {
    let (mut machine, host) = interactive(
        "port-coalesced",
        // Parks forever.
        "chip.sleep(60)",
    );

    host.send(b"one");
    for _ in 0..5 {
        machine.tick_once();
        host.send(b"more");
    }
    let raised = machine.host().events.read(Label::Peripheral);
    assert_eq!(raised.len(), 1, "five ticks should still have rung once");
    assert_eq!(raised[0].name, "neetemu:port");
}

#[test]
fn a_port_reaches_the_guest_through_the_world_file() {
    let disk = TestDisk::new("port-spec");
    expect_clean(disk.run_attached(
        Config::default(),
        &[Port::TYPE],
        r#"
        same("attached by type name", io.getType(io.getPeripherals()[1]), "neetemu:port")
    "#,
    ));
}

#[test]
fn the_host_end_survives_the_guest_finishing() {
    let (mut machine, host) = interactive(
        "port-outlives",
        r#"
        local p = io.wrapPeripheral(io.getPeripherals()[1])
        p.write("last words")
    "#,
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while machine.is_running() && std::time::Instant::now() < deadline {
        if machine.tick_once() == Outcome::Completed {
            break;
        }
    }
    assert_eq!(host.recv(), b"last words");
}
