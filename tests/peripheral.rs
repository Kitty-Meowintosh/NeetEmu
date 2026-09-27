//! The peripheral framework, against `IOAPI.java` and `PeripheralBlockEntity.java`.

mod common;

use common::{expect_clean, TestDisk};
use neetemu::events::{Label, Value};
use neetemu::host::{Clock, Host};
use neetemu::peripheral::Echo;
use neetemu::vm::Config;

fn run(name: &str, body: &str) {
    let disk = TestDisk::new(name);
    expect_clean(disk.run_attached(Config::default(), &[Echo::TYPE], body));
}

#[test]
fn nothing_is_attached_by_default() {
    let disk = TestDisk::new("io-empty");
    expect_clean(disk.run(r#"
        same("no peripherals", #io.getPeripherals(), 0)
        raises("getType", "Peripheral not found", io.getType, "00000000-0000-4000-8000-000000000000")
    "#));
}

#[test]
fn an_attached_module_is_listed_and_typed() {
    run(
        "io-listed",
        r#"
        local ids = io.getPeripherals()
        same("one peripheral", #ids, 1)
        same("its type", io.getType(ids[1]), "neetemu:echo")
        same("no tag yet", io.getTag(ids[1]), "")
        same("not a compat bridge", io.isCompatibility(ids[1]), false)
    "#,
    );
}

#[test]
fn a_wrapped_module_and_call_function_agree() {
    run(
        "io-wrapped",
        r#"
        local id = io.getPeripherals()[1]
        local p = io.wrapPeripheral(id)
        check("wrapPeripheral built the function table", type(p.echo) == "function")
        same("through the wrapper", p.echo("hi"), "hi")
        same("through callFunction", io.callFunction(id, "echo", "hi"), "hi")
        -- Both paths reach the same module, so the call counter keeps rising.
        local before = io.callFunction(id, "calls")
        p.calls()
        same("one module behind both", io.callFunction(id, "calls"), before + 2)
    "#,
    );
}

#[test]
fn a_module_event_is_named_after_its_type() {
    // `PeripheralBlockEntity.queueEvent` puts the uuid and the event name first.
    run(
        "io-event",
        r#"
        local id = io.getPeripherals()[1]
        io.callFunction(id, "ping", "payload")
        local queue = event.getQueue("Peripheral")
        same("one event", #queue, 1)
        same("named after the type", queue[1][1], "neetemu:echo")
        same("uuid first", queue[1][2], id)
        same("event name second", queue[1][3], "pong")
        same("payload after that", queue[1][4], "payload")
    "#,
    );
}

#[test]
fn tags_are_settable_and_searchable() {
    run(
        "io-tags",
        r#"
        local id = io.getPeripherals()[1]
        io.setTag(id, "left")
        same("the tag stuck", io.getTag(id), "left")
        same("found by tag", io.queryTag("left")[1], id)
        same("a tag nothing carries", #io.queryTag("right"), 0)
        raises("a blank tag", "tag cant be blank", io.queryTag, "  ")
        io.setTag(id)
        same("an absent tag clears it", io.getTag(id), "")
    "#,
    );
}

#[test]
fn query_type_namespaces_a_bare_name() {
    // `queryType` prefixes anything without a colon with the mod's own namespace.
    run(
        "io-querytype",
        r#"
        local id = io.getPeripherals()[1]
        same("found by full type", io.queryType("neetemu:echo")[1], id)
        same("a bare name looks under neetcomputers", #io.queryType("echo"), 0)
    "#,
    );
}

#[test]
fn a_module_error_reaches_the_guest_verbatim() {
    run(
        "io-error",
        r#"
        local id = io.getPeripherals()[1]
        raises("the module's own message", "echo asked to fail", io.callFunction, id, "fail")
        raises("a function it does not have", "Peripheral not found",
            io.callFunction, id, "nosuch")
    "#,
    );
}

#[test]
fn a_malformed_uuid_is_not_a_missing_one() {
    run(
        "io-uuid",
        r#"
        raises("malformed", "UUID invalidly formatted", io.getType, "nope")
        raises("well formed but absent", "Peripheral not found",
            io.getType, "ffffffff-0000-4000-8000-ffffffffffff")
    "#,
    );
}

#[test]
fn hot_plug_raises_attached_and_detached() {
    // `ComputerBlockEntity.java:186-191` puts the type first and the uuid second.
    let mut host = Host::bare(Config::default(), Clock::start());
    let uuid = host.plug_peripheral(Box::new(Echo::default()));

    let attached = host.events.take(Label::System);
    assert_eq!(attached.len(), 1);
    assert_eq!(attached[0].name, "peripheralAttached");
    assert_eq!(
        attached[0].args,
        vec![Value::Str(Echo::TYPE.into()), Value::Str(uuid.clone())]
    );

    assert!(host.unplug_peripheral(&uuid));
    let detached = host.events.take(Label::System);
    assert_eq!(detached[0].name, "peripheralDetached");
    assert_eq!(
        detached[0].args,
        vec![Value::Str(Echo::TYPE.into()), Value::Str(uuid.clone())]
    );

    assert!(
        !host.unplug_peripheral(&uuid),
        "detaching twice should fail"
    );
    assert!(host.peripherals.is_empty());
}

#[test]
fn a_module_event_wakes_a_parked_machine() {
    let mut host = Host::bare(Config::default(), Clock::start());
    let uuid = host.attach_peripheral(Box::new(Echo::default()));

    host.park_until(None);
    assert!(host.is_parked());
    host.call_peripheral(&uuid, "echo", &[])
        .expect("found")
        .expect("echo");
    assert!(
        host.is_parked(),
        "a call that raises nothing should not wake it"
    );

    host.call_peripheral(&uuid, "ping", &[])
        .expect("found")
        .expect("ping");
    assert!(!host.is_parked(), "the module's event should have woken it");
}

#[test]
fn hot_plugging_wakes_a_parked_machine() {
    let mut host = Host::bare(Config::default(), Clock::start());
    host.park_until(None);
    host.plug_peripheral(Box::new(Echo::default()));
    assert!(!host.is_parked());
}
