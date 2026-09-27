//! The mod's own network: cable segments and the access point.

use std::path::PathBuf;

use neetemu::world::{MachineSpec, PeripheralSpec, World, WorldSpec};

/// Several disks under one root, each booting its own chunk.
struct Cluster {
    root: PathBuf,
}

impl Cluster {
    fn new(name: &str, chunks: &[&str]) -> Cluster {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-disks")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        for (index, chunk) in chunks.iter().enumerate() {
            let disk = root.join((index + 1).to_string());
            std::fs::create_dir_all(disk.join("system")).expect("mkdir");
            std::fs::write(
                disk.join("build.json"),
                br#"{ "entrypoint": "system:startup.lua", "language": "Lua",
                      "partitions": [ { "path": "system", "readonly": false, "hidden": false } ] }"#,
            )
            .expect("build.json");
            std::fs::write(disk.join("system/startup.lua"), chunk).expect("startup");
        }
        Cluster { root }
    }

    /// Runs the world until every machine stops, then reads what each wrote down.
    fn run(&self, machines: Vec<MachineSpec>) -> Vec<String> {
        let count = machines.len();
        let spec = WorldSpec {
            disk_root: self.root.clone(),
            tps: None,
            internet: false,
            machines,
        };
        let mut world = World::start(&spec).expect("world");
        for _ in 0..1200 {
            if world.all_stopped() {
                break;
            }
            world.tick();
            // The clock is the host's, so a parked machine needs real time to pass.
            if world.all_parked() {
                std::thread::sleep(std::time::Duration::from_millis(4));
            }
        }
        (1..=count)
            .map(|disk| {
                std::fs::read_to_string(self.root.join(disk.to_string()).join("system/out.txt"))
                    .unwrap_or_else(|_| "<nothing written>".into())
            })
            .collect()
    }
}

fn machine(disk: u32, networks: &[&str]) -> MachineSpec {
    MachineSpec {
        disk,
        screen: None,
        batches: None,
        no_preempt: None,
        clipboard: None,
        peripheral: Vec::new(),
        share: Vec::new(),
        networks: networks.iter().map(|n| n.to_string()).collect(),
    }
}

/// Broadcasts `what` once, a moment after boot.
fn sender(what: &str) -> String {
    format!(
        r#"
        chip.sleep(0.1)
        io.broadcastLocal("{what}")
        chip.sleep(0.2)
        local out = files.open("system:/out.txt", "w", 0)
        out.write("sent")
        out.close()
        chip.shutdown()
        "#
    )
}

/// Writes down every `networkMessage` payload it hears.
const LISTENER: &str = r#"
    local heard = {}
    for _ = 1, 40 do
        for _, e in ipairs(event.getQueue("Network")) do
            if e[1] == "networkMessage" then heard[#heard + 1] = tostring(e[2]) end
        end
        chip.sleep(0.01)
    end
    local out = files.open("system:/out.txt", "w", 0)
    out.write(table.concat(heard, ","))
    out.close()
    chip.shutdown()
"#;

#[test]
fn a_machine_on_no_segment_hears_nobody() {
    let cluster = Cluster::new("net-isolated", &[&sender("one"), LISTENER]);
    let out = cluster.run(vec![machine(1, &[]), machine(2, &[])]);
    assert_eq!(out[1], "", "an unwired machine should hear nothing");
}

#[test]
fn a_segment_carries_between_its_members() {
    let cluster = Cluster::new("net-segment", &[&sender("one"), LISTENER]);
    let out = cluster.run(vec![machine(1, &["lab"]), machine(2, &["lab"])]);
    assert_eq!(out[1], "one");
}

#[test]
fn a_machine_on_another_segment_is_out_of_reach() {
    let cluster = Cluster::new("net-split", &[&sender("one"), LISTENER, LISTENER]);
    let out = cluster.run(vec![
        machine(1, &["lab"]),
        machine(2, &["lab"]),
        machine(3, &["uplink"]),
    ]);
    assert_eq!(out[1], "one", "the peer on the same segment hears it");
    assert_eq!(out[2], "", "the peer on another segment does not");
}

#[test]
fn sharing_two_segments_still_delivers_once() {
    // `isntDuplicate` keeps a receiver reachable twice off the list twice.
    let cluster = Cluster::new("net-dedupe", &[&sender("one"), LISTENER]);
    let out = cluster.run(vec![
        machine(1, &["lab", "uplink"]),
        machine(2, &["lab", "uplink"]),
    ]);
    assert_eq!(out[1], "one");
}

#[test]
fn a_sender_never_hears_itself() {
    // `scanForNetworkInternal` seeds its own position into `investigated`.
    let cluster = Cluster::new(
        "net-self",
        &[r#"
        chip.sleep(0.1)
        io.broadcastLocal("mine")
        local heard = {}
        for _ = 1, 20 do
            for _, e in ipairs(event.getQueue("Network")) do heard[#heard + 1] = tostring(e[2]) end
            chip.sleep(0.01)
        end
        local out = files.open("system:/out.txt", "w", 0)
        out.write(table.concat(heard, ","))
        out.close()
        chip.shutdown()
    "#],
    );
    let out = cluster.run(vec![machine(1, &["lab"])]);
    assert_eq!(out[0], "");
}

#[test]
fn a_table_cannot_go_over_the_network() {
    // `@Primative` filters the varargs down to what `VarType.PRIMITIVE` accepts.
    let cluster = Cluster::new(
        "net-table",
        &[r#"
        local ok, err = pcall(io.broadcastLocal, "fine", {})
        local out = files.open("system:/out.txt", "w", 0)
        out.write(tostring(ok) .. "|" .. tostring(err))
        out.close()
        chip.shutdown()
    "#],
    );
    let out = cluster.run(vec![machine(1, &["lab"])]);
    assert_eq!(out[0], "false|#2 Expected primitive, got table");
}

/// An access point at `pos`, with everything else left at its default.
fn point(disk: u32, pos: [i64; 3], options: &str) -> MachineSpec {
    let table = format!("pos = [{}, {}, {}]\n{options}", pos[0], pos[1], pos[2]);
    MachineSpec {
        disk,
        screen: None,
        batches: None,
        no_preempt: None,
        clipboard: None,
        peripheral: vec![PeripheralSpec {
            kind: "neetcomputers:access_point".into(),
            tag: None,
            options: table.parse().expect("options"),
        }],
        share: Vec::new(),
        networks: Vec::new(),
    }
}

/// Broadcasts over the air once.
const AIR_SENDER: &str = r#"
    chip.sleep(0.1)
    io.callFunction(io.getPeripherals()[1], "broadcast", "hello")
    chip.sleep(0.2)
    local out = files.open("system:/out.txt", "w", 0)
    out.write("sent")
    out.close()
    chip.shutdown()
"#;

/// Writes down the distance and payload of every `received`.
const AIR_LISTENER: &str = r#"
    local heard = {}
    for _ = 1, 40 do
        for _, e in ipairs(event.getQueue("Peripheral")) do
            if e[3] == "received" then
                heard[#heard + 1] = string.format("%.1f:%s", e[4], tostring(e[5]))
            end
        end
        chip.sleep(0.01)
    end
    local out = files.open("system:/out.txt", "w", 0)
    out.write(table.concat(heard, ","))
    out.close()
    chip.shutdown()
"#;

#[test]
fn a_point_in_range_hears_the_distance_first() {
    // `receive` puts the distance ahead of the payload.
    let cluster = Cluster::new("air-range", &[AIR_SENDER, AIR_LISTENER]);
    let out = cluster.run(vec![point(1, [0, 0, 0], ""), point(2, [3, 0, 4], "")]);
    assert_eq!(out[1], "5.0:hello");
}

#[test]
fn a_point_out_of_range_hears_nothing() {
    let cluster = Cluster::new("air-far", &[AIR_SENDER, AIR_LISTENER]);
    let out = cluster.run(vec![
        point(1, [0, 0, 0], "range = 10"),
        point(2, [100, 0, 0], ""),
    ]);
    assert_eq!(out[1], "");
}

#[test]
fn a_point_in_another_dimension_hears_nothing() {
    let cluster = Cluster::new("air-dimension", &[AIR_SENDER, AIR_LISTENER]);
    let out = cluster.run(vec![
        point(1, [0, 0, 0], ""),
        point(2, [1, 0, 0], "dimension = \"the_nether\""),
    ]);
    assert_eq!(out[1], "");
}

#[test]
fn a_point_never_hears_its_own_broadcast() {
    // `!otherPos.equals(pos)` skips a point at the same spot.
    let cluster = Cluster::new(
        "air-self",
        &[r#"
        chip.sleep(0.1)
        io.callFunction(io.getPeripherals()[1], "broadcast", "mine")
        local heard = {}
        for _ = 1, 20 do
            for _, e in ipairs(event.getQueue("Peripheral")) do
                if e[3] == "received" then heard[#heard + 1] = tostring(e[5]) end
            end
            chip.sleep(0.01)
        end
        local out = files.open("system:/out.txt", "w", 0)
        out.write(table.concat(heard, ","))
        out.close()
        chip.shutdown()
    "#],
    );
    let out = cluster.run(vec![point(1, [0, 0, 0], "")]);
    assert_eq!(out[0], "");
}

#[test]
fn range_is_readable_and_bounded() {
    let cluster = Cluster::new(
        "air-setrange",
        &[r#"
        local id = io.getPeripherals()[1]
        local said = {}
        said[#said + 1] = tostring(io.callFunction(id, "getRange"))
        io.callFunction(id, "setRange", 120)
        said[#said + 1] = tostring(io.callFunction(id, "getRange"))
        said[#said + 1] = tostring(select(2, pcall(io.callFunction, id, "setRange", 501)))
        said[#said + 1] = tostring(select(2, pcall(io.callFunction, id, "setRange", -1)))
        local out = files.open("system:/out.txt", "w", 0)
        out.write(table.concat(said, "|"))
        out.close()
        chip.shutdown()
    "#],
    );
    let out = cluster.run(vec![point(1, [0, 0, 0], "")]);
    assert_eq!(
        out[0],
        "500|120|#1 Number 501 not in range of 0-500|#1 Number -1 not in range of 0-500"
    );
}

#[test]
fn a_misspelled_option_is_refused_at_startup() {
    let cluster = Cluster::new("air-badoption", &[AIR_SENDER]);
    let spec = WorldSpec {
        disk_root: cluster.root.clone(),
        tps: None,
        internet: false,
        machines: vec![point(1, [0, 0, 0], "rnage = 10")],
    };
    let error = World::start(&spec)
        .err()
        .expect("should refuse")
        .to_string();
    assert!(error.contains("rnage"), "{error}");
    assert!(error.contains("pos, range, dimension"), "{error}");
}
