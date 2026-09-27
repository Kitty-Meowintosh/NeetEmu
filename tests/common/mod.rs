//! A disposable disk and a Lua harness, for exercising the host tables directly.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use neetemu::host::Clock;
use neetemu::vm::{Config, Outcome};
use neetemu::world::{Machine, PeripheralSpec};

/// Collects every failure instead of stopping at the first.
const HARNESS: &str = r#"
local failures = {}
local checks = 0

function check(name, cond, detail)
    checks = checks + 1
    if not cond then
        failures[#failures + 1] = name .. (detail and (": " .. tostring(detail)) or "")
    end
end

function same(name, got, want)
    check(name, got == want, "got " .. tostring(got) .. ", want " .. tostring(want))
end

--- Asserts `fn` raises, and that the message is exactly `want` when given.
function raises(name, want, fn, ...)
    local ok, err = pcall(fn, ...)
    if ok then
        failures[#failures + 1] = name .. ": expected an error, got none"
        return
    end
    checks = checks + 1
    if want and tostring(err) ~= want then
        failures[#failures + 1] = name .. ": got " .. string.format("%q", tostring(err))
            .. ", want " .. string.format("%q", want)
    end
end

function finish()
    if #failures > 0 then
        error(#failures .. "/" .. checks .. " failed:\n  " .. table.concat(failures, "\n  "), 0)
    end
end
"#;

pub struct TestDisk {
    root: PathBuf,
}

impl TestDisk {
    /// Lays out a two-partition disk under `target/test-disks/<name>`.
    pub fn new(name: &str) -> TestDisk {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-disks")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        let disk = root.join("1");
        std::fs::create_dir_all(disk.join("system/sub")).expect("mkdir");
        std::fs::create_dir_all(disk.join("rom")).expect("mkdir");

        write(
            &disk.join("build.json"),
            br#"{ "entrypoint": "system:startup.lua", "language": "Lua",
                  "partitions": [
                    { "path": "system", "readonly": false, "hidden": false },
                    { "path": "rom", "readonly": true, "hidden": false } ] }"#,
        );
        write(
            &disk.join("system/startup.lua"),
            b"-- replaced by each case\n",
        );
        write(&disk.join("system/hello.txt"), b"hello\nworld\n");
        write(&disk.join("system/noeol.txt"), b"tail");
        write(&disk.join("system/empty.txt"), b"");
        // Not valid UTF-8.
        write(
            &disk.join("system/binary.dat"),
            &[0xFF, 0xFE, 0x00, 0x01, b'z'],
        );
        write(&disk.join("system/MixedCase.txt"), b"mixed");
        write(&disk.join("system/sub/a.txt"), b"a");
        write(&disk.join("system/sub/b.txt"), b"b");
        write(&disk.join("rom/readonly.txt"), b"rom");

        TestDisk { root }
    }

    /// The directory holding the numbered disks, as `--disk-root` takes it.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.root.join("1").join(relative)
    }

    pub fn read(&self, relative: &str) -> Vec<u8> {
        std::fs::read(self.path(relative)).unwrap_or_default()
    }

    /// Runs `body` with the harness in scope, returning every failure it collected.
    pub fn run(&self, body: &str) -> Result<(), String> {
        self.run_with(Config::default(), body)
    }

    pub fn run_with(&self, config: Config, body: &str) -> Result<(), String> {
        self.run_attached(config, &[], body)
    }

    /// Runs `body` with one module of each named type already attached.
    pub fn run_attached(
        &self,
        config: Config,
        peripherals: &[&str],
        body: &str,
    ) -> Result<(), String> {
        let kinds: Vec<String> = peripherals.iter().map(|k| k.to_string()).collect();
        self.run_setup(config, body, move |host| {
            for kind in kinds {
                let spec = PeripheralSpec {
                    kind,
                    tag: None,
                    ..Default::default()
                };
                spec.attach(host).expect("attach");
            }
        })
    }

    /// Runs `body` against a machine `setup` has had a turn at.
    pub fn run_setup(
        &self,
        config: Config,
        body: &str,
        setup: impl FnOnce(&mut neetemu::host::Host),
    ) -> Result<(), String> {
        let source = format!("{HARNESS}\n{body}\nfinish()\n");
        let mut machine = Machine::start_with(
            0,
            &self.root,
            1,
            config,
            Clock::start(),
            Some(source.into_bytes()),
        )
        .map_err(|e| e.to_string())?;

        setup(machine.host());

        for _ in 0..2000 {
            // The clock is the host's, so a parked machine needs real time to pass.
            if machine.host().is_parked() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            match machine.tick_once() {
                Outcome::Ran => {}
                Outcome::Completed => return Ok(()),
                Outcome::Error(message) => return Err(message),
                other => return Err(format!("{other:?}")),
            }
        }
        Err("did not finish within the tick budget".into())
    }
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// Asserts the case passed, printing every failure it found.
pub fn expect_clean(result: Result<(), String>) {
    if let Err(report) = result {
        panic!("\n{report}\n");
    }
}
