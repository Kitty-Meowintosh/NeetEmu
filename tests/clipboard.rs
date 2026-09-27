//! `chip.getClipboard` and `chip.setClipboard`, headless, on the emulator's own buffer.

mod common;

use std::sync::{Mutex, MutexGuard};

use common::{expect_clean, TestDisk};
use neetemu::vm::Config;

/// The clipboard is one per process.
static CLIPBOARD: Mutex<()> = Mutex::new(());

fn alone() -> MutexGuard<'static, ()> {
    CLIPBOARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs `body` on a machine given the clipboard calls.
fn run(disk: &TestDisk, body: &str) {
    expect_clean(disk.run_with(
        Config {
            clipboard: true,
            ..Config::default()
        },
        body,
    ));
}

#[test]
fn the_calls_are_there_and_round_trip() {
    let _turn = alone();
    let disk = TestDisk::new("clipboard-round-trip");
    run(
        &disk,
        r#"
        check("getClipboard is a function", type(chip.getClipboard) == "function")
        check("setClipboard is a function", type(chip.setClipboard) == "function")
        chip.setClipboard("meow")
        same("reads back what was copied", chip.getClipboard(), "meow")
        chip.setClipboard("")
        same("an empty copy reads back empty", chip.getClipboard(), "")
    "#,
    );
}

#[test]
fn a_copy_survives_until_the_next_one() {
    let _turn = alone();
    let disk = TestDisk::new("clipboard-replace");
    run(
        &disk,
        r#"
        chip.setClipboard("first")
        chip.setClipboard("second")
        same("the later copy wins", chip.getClipboard(), "second")
    "#,
    );
}

#[test]
fn the_clipboard_is_one_thing_across_machines() {
    let _turn = alone();
    let sender = TestDisk::new("clipboard-sender");
    run(&sender, r#"chip.setClipboard("passed along")"#);

    let receiver = TestDisk::new("clipboard-receiver");
    run(
        &receiver,
        r#"
        same("the other machine's copy is here", chip.getClipboard(), "passed along")
    "#,
    );
}

#[test]
fn copying_something_that_is_not_a_string_raises() {
    let _turn = alone();
    let disk = TestDisk::new("clipboard-argument");
    run(
        &disk,
        r##"
        raises("a table is refused", "#1 text: string expected", chip.setClipboard, {})
        raises("nothing is refused", "#1 text: string expected", chip.setClipboard)
    "##,
    );
}

#[test]
fn the_calls_are_withheld_by_default() {
    let _turn = alone();
    let disk = TestDisk::new("clipboard-withheld");
    expect_clean(disk.run(
        r#"
        same("getClipboard is absent", type(chip.getClipboard), "nil")
        same("setClipboard is absent", type(chip.setClipboard), "nil")
    "#,
    ));
}
