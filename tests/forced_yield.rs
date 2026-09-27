//! The forced-yield machinery, exercised directly.

use neetemu::host::{Clock, Host};
use neetemu::vm::{Config, Outcome, Vm};

fn vm(config: Config, src: &str) -> Vm {
    let mut vm = Vm::new(Host::bare(config, Clock::start()), config).expect("lua state");
    vm.load_entrypoint(src.as_bytes(), "Lua").expect("load");
    vm
}

/// Drives a chunk for at most `max_ticks`, returning the first non-`Ran` outcome.
fn run(config: Config, src: &str, max_ticks: usize) -> (Outcome, usize) {
    let mut vm = vm(config, src);
    for tick in 1..=max_ticks {
        match vm.tick() {
            Outcome::Ran => {}
            other => return (other, tick),
        }
    }
    (Outcome::Ran, max_ticks)
}

/// A one-ticket tick, so each tick relays a spinning child exactly once.
fn thin() -> Config {
    Config {
        batches_per_tick: 1,
        ..Config::default()
    }
}

#[test]
fn spinning_main_chunk_is_preempted_not_hung() {
    let (outcome, ticks) = run(Config::default(), "while true do end", 3);
    assert_eq!(outcome, Outcome::Ran);
    assert_eq!(ticks, 3);
}

#[test]
fn spinning_child_coroutine_relays_then_dies() {
    // `auxresume` relays once per host yield, so the cap needs LUA_MAXRESUMERELAYS ticks.
    let (outcome, ticks) = run(
        thin(),
        "local co = coroutine.create(function() while true do end end)
         coroutine.resume(co)",
        1100,
    );
    match outcome {
        Outcome::Error(msg) => {
            assert!(
                msg.contains("script exceeded instruction budget without yielding"),
                "unexpected error: {msg}"
            );
            assert_eq!(ticks, 1001, "the relay cap is 1000");
        }
        other => panic!("expected the relay cap to fire, got {other:?}"),
    }
}

#[test]
fn main_chunk_may_yield() {
    let (outcome, _) = run(Config::default(), "coroutine.yield() coroutine.yield()", 3);
    assert_eq!(outcome, Outcome::Completed);
}

#[test]
fn state_survives_preemption() {
    let mut vm = vm(
        thin(),
        "local n = 0 for _ = 1, 100000 do n = n + 1 end done = n",
    );
    let mut ticks = 0;
    loop {
        ticks += 1;
        assert!(ticks < 10_000, "never completed");
        if vm.tick() == Outcome::Completed {
            break;
        }
    }
    assert!(ticks > 1, "the loop should not have fitted in one tick");
    assert_eq!(vm.global_number("done"), Some(100_000.0));
}

#[test]
fn a_child_coroutine_that_yields_is_not_relayed() {
    let (outcome, _) = run(
        Config::default(),
        "local co = coroutine.create(function() coroutine.yield('real') end)
         local ok, v = coroutine.resume(co)
         assert(ok and v == 'real', 'forced yield leaked as a result')",
        4,
    );
    assert_eq!(outcome, Outcome::Completed);
}

#[test]
fn tickets_carry_over_between_ticks() {
    let mut vm = vm(Config::default(), "while true do end");
    vm.tick();
    assert!(
        vm.host().tickets <= 0,
        "a tick should exhaust its allowance"
    );
    vm.tick();
    assert!(vm.host().tickets <= 0);
}

#[test]
fn the_kernels_own_hook_preempts_a_child_without_the_host_seeing_it() {
    // A per-coroutine count hook that yields "PREEMPT".
    let (outcome, _) = run(
        Config::default(),
        "local co = coroutine.create(function() while true do end end)
         debug.sethook(co, function() coroutine.yield('PREEMPT') end, '', 500)
         local ok, v = coroutine.resume(co)
         assert(ok and v == 'PREEMPT', 'child hook did not surface: ' .. tostring(v))",
        4,
    );
    assert_eq!(outcome, Outcome::Completed);
}

#[test]
fn chunkname_is_the_language_field() {
    let (outcome, _) = run(Config::default(), "error('boom')", 1);
    match outcome {
        Outcome::Error(msg) => assert!(
            msg.starts_with("[string \"Lua\"]"),
            "chunkname should be the language field, got {msg}"
        ),
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn a_tick_hands_control_back_when_its_slice_runs_out() {
    // A slice lets the host take a turn before the tick ends.
    let config = Config {
        slice: Some(std::time::Duration::from_millis(10)),
        ..Config::default()
    };
    let mut vm = vm(config, "while true do end");

    let started = std::time::Instant::now();
    let outcome = vm.tick();

    assert_eq!(outcome, Outcome::Ran);
    assert!(vm.host().mid_tick, "the tick should have stopped part-way");
    assert!(
        vm.host().tickets > 0,
        "the tick spent its whole budget anyway"
    );
    assert!(
        started.elapsed().as_millis() < 500,
        "took {:?}",
        started.elapsed()
    );
}

#[test]
fn a_sliced_tick_is_finished_by_the_next_call() {
    let config = Config {
        slice: Some(std::time::Duration::from_millis(5)),
        ..Config::default()
    };
    let mut vm = vm(config, "while true do end");

    vm.tick();
    let carried = vm.host().tickets;
    assert!(vm.host().mid_tick);

    // The next call resumes the same tick.
    vm.tick();
    assert!(
        vm.host().tickets < carried,
        "the tick was granted a fresh budget"
    );
}

#[test]
fn an_unsliced_tick_still_spends_its_whole_budget() {
    let mut vm = vm(Config::default(), "while true do end");
    vm.tick();
    assert!(!vm.host().mid_tick);
    assert!(vm.host().tickets <= 0);
}
