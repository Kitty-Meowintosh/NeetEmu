//! YSLua on its own, with no host tables.

use neetemu::host::{Clock, Host};
use neetemu::vm::{Config, Outcome, Vm};

/// Runs a chunk with nothing but stock Lua, to completion or `max_ticks`.
fn run(config: Config, src: &str, max_ticks: usize) -> Outcome {
    let mut vm = Vm::new(Host::bare(config, Clock::start()), config).expect("lua state");
    vm.load_entrypoint(src.as_bytes(), "Lua").expect("load");
    for _ in 0..max_ticks {
        match vm.tick() {
            Outcome::Ran => {}
            other => return other,
        }
    }
    Outcome::Ran
}

/// Asserts the chunk runs to completion; its own `assert`s carry the message.
fn ok(src: &str) {
    ok_with(Config::default(), src);
}

fn ok_with(config: Config, src: &str) {
    match run(config, src, 4000) {
        Outcome::Completed => {}
        Outcome::Ran => panic!("did not finish within the tick budget"),
        other => panic!("{other:?}"),
    }
}

/// A tiny tick, so the host preempts constantly without tripping the relay cap.
fn shredder() -> Config {
    Config {
        batches_per_tick: 2,
        instructions_per_batch: 97,
        ..Config::default()
    }
}

#[test]
fn resume_delivers_every_argument() {
    ok(r#"
        local co = coroutine.create(function(...)
            local got = table.pack(...)
            assert(got.n == 3, "first resume lost arguments: " .. got.n)
            local a, b = coroutine.yield()
            assert(a == "reply" and b == 42, "second resume lost values")
            return "done"
        end)
        assert(coroutine.resume(co, 1, 2, 3))
        local live, result = coroutine.resume(co, "reply", 42)
        assert(live and result == "done", "final return lost")
    "#);
}

#[test]
fn a_yielded_value_survives_host_preemption() {
    // A syscall-shaped round trip, spinning so a forced yield lands inside it.
    ok_with(
        shredder(),
        r#"
        local co = coroutine.create(function()
            for i = 1, 40 do
                local ok, payload = coroutine.yield("SYSCALL", i)
                assert(ok == true, "reply " .. i .. " lost its status: " .. tostring(ok))
                assert(type(payload) == "table", "reply " .. i .. " lost its table: " .. tostring(payload))
                assert(payload[1] == i * 7, "reply " .. i .. " corrupted: " .. tostring(payload[1]))
            end
            return "clean"
        end)
        -- The first resume feeds the function's parameters, so prime it to the first yield.
        local live, trap, id = coroutine.resume(co)
        for i = 1, 40 do
            assert(live, "resume " .. i .. " failed: " .. tostring(trap))
            assert(trap == "SYSCALL", "trap " .. i .. " garbled: " .. tostring(trap))
            assert(id == i, "id " .. i .. " garbled: " .. tostring(id))
            live, trap, id = coroutine.resume(co, true, { i * 7 })
        end
        assert(live and trap == "clean", "tail lost: " .. tostring(trap))
    "#,
    );
}

#[test]
fn nested_resume_relays_without_losing_values() {
    ok_with(
        shredder(),
        r#"
        local inner = coroutine.create(function()
            for i = 1, 20 do
                local v = coroutine.yield("inner", i)
                assert(v == i * 3, "inner reply " .. i .. " corrupted: " .. tostring(v))
            end
            return "inner done"
        end)
        local outer = coroutine.create(function()
            -- Prime inner past its parameter list, then reply to each yield in turn.
            local live, tag, n = coroutine.resume(inner)
            for i = 1, 20 do
                assert(live, "inner resume failed: " .. tostring(tag))
                assert(tag == "inner" and n == i, "relay corrupted: " .. tostring(tag) .. "/" .. tostring(n))
                coroutine.yield("outer", i)
                live, tag, n = coroutine.resume(inner, i * 3)
            end
            return "outer done"
        end)
        for i = 1, 20 do
            local live, tag, n = coroutine.resume(outer)
            assert(live, "outer resume failed: " .. tostring(tag))
            assert(tag == "outer" and n == i, "outer corrupted: " .. tostring(tag) .. "/" .. tostring(n))
        end
        local live, tail = coroutine.resume(outer)
        assert(live and tail == "outer done", "outer tail lost: " .. tostring(tail))
    "#,
    );
}

#[test]
fn a_hook_yield_does_not_swallow_the_next_resume_values() {
    // A thread preempted from its own count hook, then resumed with a reply that must survive.
    ok(r#"
        local co = coroutine.create(function()
            local spin = 0
            for _ = 1, 20000 do spin = spin + 1 end
            local a, b = coroutine.yield("ASK")
            assert(a == true and b == "answer", "reply lost after a hook yield: "
                .. tostring(a) .. "/" .. tostring(b))
            return "ok"
        end)
        debug.sethook(co, function() coroutine.yield("PREEMPT") end, "", 500)

        local trap
        repeat
            local live, t = coroutine.resume(co)
            assert(live, "resume failed: " .. tostring(t))
            trap = t
        until trap ~= "PREEMPT"
        assert(trap == "ASK", "expected ASK, got " .. tostring(trap))

        debug.sethook(co)
        local live, result = coroutine.resume(co, true, "answer")
        assert(live and result == "ok", "tail lost: " .. tostring(result))
    "#);
}

#[test]
fn pcall_survives_a_yield_across_it() {
    // A yield from inside a pcall.
    ok_with(
        shredder(),
        r#"
        local co = coroutine.create(function()
            for i = 1, 30 do
                local ok, a, b = pcall(function()
                    return coroutine.yield("through-pcall", i)
                end)
                assert(ok, "pcall failed on " .. i .. ": " .. tostring(a))
                assert(a == true, "status lost through pcall on " .. i .. ": " .. tostring(a))
                assert(b == i * 2, "value lost through pcall on " .. i .. ": " .. tostring(b))
            end
            return "ok"
        end)
        local live, tag = coroutine.resume(co)
        for i = 1, 30 do
            assert(live, "resume failed: " .. tostring(tag))
            assert(tag == "through-pcall", "tag garbled: " .. tostring(tag))
            live, tag = coroutine.resume(co, true, i * 2)
        end
        assert(live and tag == "ok", "tail lost: " .. tostring(tag))
    "#,
    );
}

#[test]
fn an_error_object_crosses_resume_intact() {
    // A nil error object is what surfaces as Lua's `<no error object>`.
    ok(r#"
        local co = coroutine.create(function() error("the reason") end)
        local live, err = coroutine.resume(co)
        assert(live == false, "expected failure")
        assert(type(err) == "string", "error object became a " .. type(err))
        assert(err:find("the reason"), "error text lost: " .. tostring(err))
    "#);
}

#[test]
fn coroutine_status_is_honest_after_preemption() {
    ok_with(
        shredder(),
        r#"
        local co = coroutine.create(function()
            local n = 0
            for _ = 1, 5000 do n = n + 1 end
            coroutine.yield()
            for _ = 1, 5000 do n = n + 1 end
            return n
        end)
        coroutine.resume(co)
        assert(coroutine.status(co) == "suspended", "status: " .. coroutine.status(co))
        local live, n = coroutine.resume(co)
        assert(live and n == 10000, "count corrupted: " .. tostring(n))
        assert(coroutine.status(co) == "dead", "status: " .. coroutine.status(co))
    "#,
    );
}

#[test]
fn memoryused_is_callable_on_the_running_thread() {
    ok(r#"
        local n = coroutine.memoryused(0)
        assert(type(n) == "number", "memoryused returned a " .. type(n))
        assert(n > 0, "memoryused returned " .. tostring(n))
    "#);
}

#[test]
fn table_pack_and_unpack_round_trip_with_holes() {
    // Arguments round-tripped through `table.pack`/`unpack`.
    ok(r#"
        local packed = table.pack(1, nil, 3)
        assert(packed.n == 3, "pack lost the count: " .. tostring(packed.n))
        local a, b, c = table.unpack(packed, 1, packed.n)
        assert(a == 1 and b == nil and c == 3, "unpack corrupted a hole")
    "#);
}

#[test]
fn deep_require_style_nesting_does_not_corrupt_returns() {
    ok_with(
        shredder(),
        r#"
        local function nest(depth)
            if depth == 0 then
                local v = coroutine.yield("bottom")
                return v
            end
            local got = nest(depth - 1)
            return got + 1
        end
        local co = coroutine.create(function() return nest(40) end)
        local live, tag = coroutine.resume(co)
        assert(live and tag == "bottom", "descent lost: " .. tostring(tag))
        local done, result = coroutine.resume(co, 100)
        assert(done, "unwind failed: " .. tostring(result))
        assert(result == 140, "unwind corrupted: " .. tostring(result))
    "#,
    );
}
