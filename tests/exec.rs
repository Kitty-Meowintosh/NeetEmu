//! `neetemu exec` against a guest written here, speaking the protocol in `docs/exec.md`.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::TestDisk;
use neetemu::harness::{BootSpec, Harness, BOOT_TIMEOUT};

const TIMEOUT: Duration = Duration::from_secs(30);

/// Knows `echo <text>`, `fail` and `cat`; anything else is not found.
const GUEST: &str = r#"
local p = io.wrapPeripheral(io.getPeripherals()[1])
local buffer = ""

local function send(kind, payload)
    p.write(kind .. string.pack(">I4", #payload) .. payload)
end

local function frame()
    while true do
        if #buffer >= 5 then
            local len = string.unpack(">I4", buffer, 2)
            if #buffer >= 5 + len then
                local kind, payload = buffer:sub(1, 1), buffer:sub(6, 5 + len)
                buffer = buffer:sub(6 + len)
                return kind, payload
            end
        end
        local data = p.read(4096)
        if data == "" then
            local _, open = p.poll()
            if not open then return nil end
            chip.sleep()
            event.clear("Peripheral")
        else
            buffer = buffer .. data
        end
    end
end

send("R", "exec")
while true do
    local kind, statement = frame()
    if kind == nil then break end
    if kind == "X" then
        local text = statement:match("^echo (.*)$")
        if text then
            send("O", text .. "\n")
            send("S", "0")
        elseif statement == "fail" then
            send("E", "it failed\n")
            send("S", "3")
        elseif statement == "cat" then
            while true do
                local k, data = frame()
                if k == nil or k == "Z" then break end
                if k == "I" then send("O", data) end
            end
            send("S", "0")
        else
            send("E", statement .. ": not found\n")
            send("S", "127")
        end
    end
end
"#;

fn disk(name: &str) -> TestDisk {
    let disk = TestDisk::new(name);
    std::fs::write(disk.path("system/startup.lua"), GUEST).expect("write the guest");
    disk
}

fn boot(disk: &TestDisk, spec: BootSpec) -> Harness {
    let mut harness = Harness::start(disk.root(), 1, spec).expect("boot");
    assert_eq!(harness.wait_ready(BOOT_TIMEOUT).expect("ready"), "exec");
    harness
}

#[test]
fn statements_run_one_after_another() {
    let disk = disk("exec-statements");
    let mut harness = boot(&disk, BootSpec::default());

    let echoed = harness.exec("echo hi", TIMEOUT).expect("echo");
    assert_eq!(echoed.status, 0);
    assert_eq!(echoed.stdout_text(), "hi\n");
    assert_eq!(echoed.stderr_text(), "");

    let failed = harness.exec("fail", TIMEOUT).expect("fail");
    assert_eq!(failed.status, 3);
    assert_eq!(failed.stdout_text(), "");
    assert_eq!(failed.stderr_text(), "it failed\n");

    let again = harness.exec("echo again", TIMEOUT).expect("echo");
    assert_eq!(again.stdout_text(), "again\n");
    harness.close();
}

#[test]
fn standard_input_reaches_the_guest() {
    let disk = disk("exec-stdin");
    let mut harness = boot(&disk, BootSpec::default());

    harness.begin("cat");
    harness.send_stdin(b"one\ntwo\n");
    harness.end_stdin();

    let mut collected = Vec::new();
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match harness
            .pump(&mut |frame| collected.extend_from_slice(&frame.payload))
            .expect("pump")
        {
            Some(status) => break status,
            None if Instant::now() >= deadline => panic!("cat did not finish"),
            None => harness.step().expect("tick"),
        }
    };

    assert_eq!(status, 0);
    assert_eq!(String::from_utf8_lossy(&collected), "one\ntwo\n");
    harness.close();
}

#[test]
fn a_tick_rate_paces_the_harness() {
    let disk = disk("exec-paced");
    let mut harness = boot(
        &disk,
        BootSpec {
            tps: Some(20.0),
            ..BootSpec::default()
        },
    );
    assert_eq!(harness.world().period, Duration::from_millis(50));

    let started = Instant::now();
    for _ in 0..10 {
        harness.step().expect("tick");
    }
    assert!(
        started.elapsed() >= Duration::from_millis(400),
        "ten ticks at 20 tps took {:?}",
        started.elapsed()
    );
}

#[test]
fn the_harness_runs_unpaced_without_one() {
    let disk = disk("exec-unpaced");
    let mut harness = boot(&disk, BootSpec::default());
    assert_eq!(harness.world().period, neetemu::vm::TICK);

    let started = Instant::now();
    for _ in 0..10 {
        harness.step().expect("tick");
    }
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "ten unpaced ticks took {:?}",
        started.elapsed()
    );
}

fn neetemu_exec(disk: &TestDisk, statement: &str, stdin: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_neetemu"))
        .args(["exec", "--disk-root"])
        .arg(disk.root())
        .args(["--disk", "1", statement])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn neetemu");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    child.wait_with_output().expect("neetemu exec")
}

#[test]
fn the_command_exits_with_the_guests_status() {
    let disk = disk("exec-command");

    let echoed = neetemu_exec(&disk, "echo hi", b"");
    assert_eq!(echoed.status.code(), Some(0));
    assert_eq!(echoed.stdout, b"hi\n");

    let missing = neetemu_exec(&disk, "nosuchcommand", b"");
    assert_eq!(missing.status.code(), Some(127));
    assert_eq!(missing.stderr, b"nosuchcommand: not found\n");

    let piped = neetemu_exec(&disk, "cat", b"one\ntwo\n");
    assert_eq!(piped.status.code(), Some(0));
    assert_eq!(piped.stdout, b"one\ntwo\n");
}
