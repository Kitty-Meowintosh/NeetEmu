//! `chip.sleep` and the event-driven tick loop, where a parked machine is not resumed at all.

mod common;

use common::{expect_clean, TestDisk};

#[test]
fn sleep_exists_and_reports_why_it_woke() {
    let disk = TestDisk::new("sleep-basic");
    expect_clean(disk.run(
        r#"
        check("chip.sleep is a function", type(chip.sleep) == "function", type(chip.sleep))
        same("a deadline times out", chip.sleep(0.1), "timeout")
        same("zero sleeps immediately", chip.sleep(0), "timeout")
        same("negative sleeps immediately", chip.sleep(-1), "timeout")
    "#,
    ));
}

#[test]
fn sleeping_advances_emulated_time_by_the_requested_amount() {
    let disk = TestDisk::new("sleep-clock");
    expect_clean(disk.run(
        r#"
        local before = chip.getTime()
        chip.sleep(0.25)
        local slept = chip.getTime() - before
        -- The wake lands on a tick boundary, so allow one tick of overshoot.
        check("slept about a quarter second", slept >= 0.25 and slept < 0.32, slept)
    "#,
    ));
}

#[test]
fn a_queued_event_cuts_a_sleep_short() {
    let disk = TestDisk::new("sleep-event");
    // The machine queues its own event, so the very next tick must wake it.
    expect_clean(disk.run(
        r#"
        event.queueEvent("User", "poke")
        -- An event is already pending, so a park ends at once.
        same("woke on the pending event", chip.sleep(10), "event")
        local q = event.getQueue("User")
        same("the event survived the wake", q[1][1], "poke")
    "#,
    ));
}

#[test]
fn sleeping_with_no_deadline_parks_until_an_event() {
    let disk = TestDisk::new("sleep-indefinite");
    expect_clean(disk.run(
        r#"
        event.queueEvent("System", "ready")
        same("woke on an event", chip.sleep(), "event")
    "#,
    ));
}

#[test]
fn a_parked_machine_burns_no_instructions() {
    // The tick loop skips a parked machine entirely.
    use neetemu::host::Clock;
    use neetemu::vm::{Config, Outcome};
    use neetemu::world::Machine;

    let disk = TestDisk::new("sleep-idle");
    let config = Config::default();
    let mut machine = Machine::start_with(
        0,
        disk.root(),
        1,
        config,
        Clock::start(),
        Some(b"chip.sleep(0.2) done = 1".to_vec()),
    )
    .expect("machine");

    // The first tick arms the park.
    assert_eq!(machine.tick_once(), Outcome::Ran);
    assert!(machine.host().is_parked(), "the machine should be parked");

    // Ticks taken before the deadline must leave the budget untouched.
    let before = machine.host().tickets;
    for _ in 0..20 {
        assert_eq!(machine.tick_once(), Outcome::Ran);
    }
    assert!(
        machine.host().is_parked(),
        "still parked before the deadline"
    );
    assert_eq!(
        machine.host().tickets,
        before,
        "a parked machine spent tickets, so it was resumed"
    );

    // Letting the deadline arrive wakes it and finishes the chunk.
    let mut finished = false;
    for _ in 0..200 {
        if machine.tick_once() == Outcome::Completed {
            finished = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(finished, "never woke from the park");
}

#[test]
fn sleep_relays_out_of_a_nested_coroutine() {
    let disk = TestDisk::new("sleep-nested");
    // A park travels back out through `coroutine.resume`.
    expect_clean(disk.run(
        r#"
        local co = coroutine.create(function()
            local reason = chip.sleep(0.1)
            return "slept:" .. tostring(reason)
        end)
        local live, result = coroutine.resume(co)
        check("resume survived the park", live, result)
        same("the park did not leak a trap", result, "slept:timeout")
        same("the coroutine finished", coroutine.status(co), "dead")
    "#,
    ));
}

#[test]
fn a_polling_loop_can_be_replaced_by_a_park() {
    let disk = TestDisk::new("sleep-poll");
    // An idle loop that parks when the call exists.
    expect_clean(disk.run(
        r#"
        local pumped = 0
        local function idle()
            if (chip.sleep) then
                return chip.sleep(0.005)
            end
            coroutine.yield()
            return "timeout"
        end

        for _ = 1, 10 do
            local why = idle()
            check("idle returned a reason", why == "timeout" or why == "event", why)
            pumped = pumped + 1
        end
        same("pumped ten times", pumped, 10)
    "#,
    ));
}
