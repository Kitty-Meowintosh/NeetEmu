//! `--share`: a host directory the guest finds as one more partition on `drive0`.

mod common;

use std::path::PathBuf;

use common::{expect_clean, TestDisk};
use neetemu::disk::Share;
use neetemu::host::Clock;
use neetemu::vm::Config;
use neetemu::world::Machine;

fn share(name: &str, path: &std::path::Path, readonly: bool) -> Share {
    Share {
        name: name.into(),
        path: path.to_path_buf(),
        readonly,
    }
}

/// Runs `body` on a machine carrying one share over `dir`.
fn with_share(name: &str, dir: &std::path::Path, readonly: bool, body: &str) -> Result<(), String> {
    let disk = TestDisk::new(name);
    let attached = share("Share", dir, readonly);
    disk.run_setup(Config::default(), body, move |host| {
        host.disks.attach_share(&attached).expect("attach");
    })
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("neetemu-share-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

#[test]
fn a_share_lists_as_a_partition_and_reads_the_host_directory() {
    let dir = scratch("read");
    std::fs::write(dir.join("hello.txt"), "from the host").expect("write");

    expect_clean(with_share(
        "share-read",
        &dir,
        false,
        r#"
        local names = {}
        for _, name in ipairs(files.getPartitions(0)) do names[name] = true end
        check("share listed", names["Share"])
        check("system still listed", names["system"])

        local info = files.getPartition("Share", 0)
        same("name", info.name, "Share")
        same("readonly", info.readonly, false)
        same("hidden", info.hidden, false)

        check("exists", files.exists("Share:/hello.txt", 0))
        check("isFile", files.isFile("Share:/hello.txt", 0))
        local handle = files.open("Share:/hello.txt", "r", 0)
        same("contents", handle.read("a"), "from the host")
        handle.close()

        same("children", files.getChildren("Share:/", 0)[1], "hello.txt")    "#,
    ));
}

#[test]
fn a_write_reaches_the_host_and_a_read_only_share_refuses_one() {
    let writable = scratch("write");
    let frozen = scratch("frozen");
    std::fs::write(frozen.join("kept.txt"), "kept").expect("write");

    expect_clean(with_share(
        "share-write",
        &writable,
        false,
        r#"
        local handle = files.open("Share:/made.txt", "w", 0)
        handle.write("from the guest")
        handle.close()
        check("makeDir", files.makeDir("Share:/sub", 0))    "#,
    ));
    assert_eq!(
        std::fs::read_to_string(writable.join("made.txt")).expect("read"),
        "from the guest"
    );
    assert!(writable.join("sub").is_dir());

    expect_clean(with_share(
        "share-frozen",
        &frozen,
        true,
        r#"
        same("readonly", files.getPartition("Share", 0).readonly, true)
        raises("write refused", "Access denied", files.open, "Share:/new.txt", "w", 0)
        check("makeDir refused", not files.makeDir("Share:/sub", 0))
        check("delete refused", not files.delete("Share:/kept.txt", 0))
        check("still readable", files.exists("Share:/kept.txt", 0))    "#,
    ));
    assert!(frozen.join("kept.txt").exists());
}

#[test]
fn a_share_answers_no_partition_edits_and_never_leaves_its_directory() {
    let dir = scratch("escape");
    std::fs::create_dir_all(dir.join("inside")).expect("dir");
    std::fs::write(dir.join("inside/deep.txt"), "deep").expect("write");
    let outside = scratch("outside");
    std::fs::write(outside.join("secret.txt"), "secret").expect("write");
    let _ = std::os::unix::fs::symlink(&outside, dir.join("away"));

    expect_clean(with_share(
        "share-escape",
        &dir,
        false,
        r#"
        check("deletePartition", not files.deletePartition("Share", 0))
        check("setPartitionHidden", not files.setPartitionHidden("Share", true, 0))
        check("setPartitionReadOnly", not files.setPartitionReadOnly("Share", 0))
        check("createPartition", not files.createPartition("Share", 0))
        check("still there", files.getPartition("Share", 0) ~= nil)

        check("no symlink escape", not files.exists("Share:/away/secret.txt", 0))
        check("inside is reachable", files.exists("Share:/inside/deep.txt", 0))    "#,
    ));
}

#[test]
fn a_share_stays_attached_across_a_reboot() {
    let dir = scratch("reboot");
    std::fs::write(dir.join("kept.txt"), "kept").expect("write");
    let disk = TestDisk::new("share-reboot");
    let mut machine =
        Machine::start(0, disk.root(), 1, Config::default(), Clock::start()).expect("start");
    machine
        .install(Vec::new(), vec![share("Share", &dir, false)], false)
        .expect("install");

    assert!(machine.host().disks.boot().entry("Share").is_some());
    machine.reboot().expect("reboot");
    assert!(machine.host().disks.boot().entry("Share").is_some());
    assert!(machine
        .host()
        .disks
        .boot()
        .resolve("Share:/kept.txt")
        .is_some());
}

#[test]
fn a_share_that_breaks_a_rule_stops_the_machine_and_names_itself() {
    let dir = scratch("rules");
    let disk = TestDisk::new("share-rules");

    let refused = |bad: Share| -> String {
        let mut machine =
            Machine::start(0, disk.root(), 1, Config::default(), Clock::start()).expect("start");
        machine
            .install(Vec::new(), vec![bad], false)
            .expect_err("should refuse")
            .to_string()
    };

    let message = refused(share("system", &dir, false));
    assert!(
        message.contains("\"system\"") && message.contains("already a partition"),
        "{message}"
    );

    let message = refused(share("Sh4re", &dir, false));
    assert!(
        message.contains("\"Sh4re\"") && message.contains("ASCII letters"),
        "{message}"
    );

    let message = refused(share("Gone", &dir.join("nowhere"), false));
    assert!(
        message.contains("\"Gone\"") && message.contains("not a directory"),
        "{message}"
    );

    let mut machine =
        Machine::start(0, disk.root(), 1, Config::default(), Clock::start()).expect("start");
    let message = machine
        .install(
            Vec::new(),
            vec![share("Twice", &dir, false), share("twice", &dir, false)],
            false,
        )
        .expect_err("should refuse")
        .to_string();
    assert!(
        message.contains("\"twice\"") && message.contains("already a partition"),
        "{message}"
    );
}

#[test]
fn the_flag_spells_a_share_as_name_path_and_an_optional_ro() {
    let share = Share::parse("Share=/Users/me/Share").expect("parse");
    assert_eq!(share.name, "Share");
    assert_eq!(share.path, PathBuf::from("/Users/me/Share"));
    assert!(!share.readonly);

    let share = Share::parse("Src=/Users/me/src,ro").expect("parse");
    assert_eq!(share.path, PathBuf::from("/Users/me/src"));
    assert!(share.readonly);

    assert!(Share::parse("Share").is_err());
    assert!(Share::parse("Share=").is_err());
}
