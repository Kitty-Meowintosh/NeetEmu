# NeetEmu

NeetEmu is an emulator for NEET (NeetComputers)
computers, the programmable computers from a Minecraft mod, that runs on an ordinary
computer without Minecraft. It runs the mod's own Lua interpreter,
[YSLua](https://codeberg.org/SpartanSoftware/Neet-YSLua), and reimplements in Rust the host
API the mod gives its computers.

It boots whatever a NEET computer boots: point it at the computer's directory in a world
save.

## Building

You need Rust 1.87 or newer and a C compiler. [SDL3](https://www.libsdl.org/) is optional:
with it you get a window, and without it NeetEmu only runs `--headless`.

```sh
git clone --recursive https://github.com/Kitty-Meowintosh/NeetEmu
cd NeetEmu
cargo build --release        # the binary is target/release/neetemu
cargo test
```

`build.rs` looks for SDL3 in `SDL3_DIR` first, then asks `pkg-config`, then checks
`/opt/homebrew`, `/usr/local` and `/usr`. NeetEmu is developed on macOS and should
build on Linux.

## Disks

A disk uses the same layout as a NEET world save: one numbered directory per computer,
with a `build.json` at its top and one directory per partition.

```
disks/
  33/
    build.json
    system/
    user/
```

`--disk-root` names the directory holding the numbered disks, and `--disk` picks one.
When you run `neetemu` from inside a disk directory, with neither flag, it boots that disk.

## Running

```sh
neetemu --disk-root disks --disk 33                  # in a window
neetemu --disk-root disks --disk 33 --headless --ticks 200
```

The window is 800x600 and redraws whenever the guest calls `screen.draw()`. Keyboard and
mouse input reach the guest as NEET's own `keyPressed`, `keyReleased` and `mouse*` events.
Without `--ticks` a headless run lasts until every machine stops.

| Flag | Effect |
| --- | --- |
| `--disk-root <dir>` | directory holding the numbered disks, `.` by default |
| `--disk <n>` | add a machine booting that disk; repeatable |
| `--world <file>` | load the machines from a world file |
| `--ticks <n>` | stop after n ticks |
| `--tps <n>` | ticks per second, 20 by default as in the mod |
| `--batches <n>` | tickets granted per tick, 3750 by default |
| `--headless` | emulate the screen without opening a window |
| `--peripheral <type>[=<tag>]` | attach a module to the last `--disk` |
| `--share <Name>=<path>[,ro]` | show the last `--disk` a host directory |
| `--network <name>` | wire the last `--disk` to a cable segment |
| `--internet` | let machines reach the network |
| `--no-preempt` | never force a yield from the instruction hook |
| `--clipboard` | give machines `chip.getClipboard` and `chip.setClipboard` |
| `--eval <lua>` | run this chunk in place of the disk's entrypoint |
| `--eval-file <path>` | run this file in place of the disk's entrypoint |

## Running one command

`neetemu exec` boots a disk without a window, hands one statement to the guest, streams
its output, and exits with the guest's status:

```sh
$ neetemu exec --disk-root disks --disk 33 'echo hi'
hi
$ printf 'one\ntwo\n' | neetemu exec --disk-root disks --disk 33 'cat'
one
two
```

This works with any guest that answers the protocol in [`docs/exec.md`](docs/exec.md): a
program that claims the emulator's `neetemu:port` device and runs what it is sent. The
emulator passes output through byte for byte. `exec` takes `--share`, `--peripheral`,
`--network`, `--internet`, `--tps`, `--batches`, `--no-preempt` and `--clipboard`, plus:

| Flag | Effect |
| --- | --- |
| `--timeout <s>` | seconds the statement may take, 60 by default |
| `--boot-timeout <s>` | seconds the guest has to claim the port, 60 by default |

## Several machines

A world file describes the machines in a run, the cable segments they are wired to, and
the modules each one carries:

```toml
disk-root = "disks"      # relative to this file
internet = true

[[machine]]
disk = 33
networks = ["lab"]

[[machine]]
disk = 34
networks = ["lab"]
screen = [400, 300]

[[machine.peripheral]]
type = "neetcomputers:access_point"
options = { pos = [0, 64, 0], range = 100 }

[[machine.share]]
name = "Share"
path = "/home/me/Share"
readonly = true
```

`io.broadcastLocal` reaches every other machine on the same segment. The peripheral types
are `neetcomputers:access_point`, `neetemu:echo` (a test module) and `neetemu:port`.

## Shared folders

`--share Name=/path` attaches a host directory to the machine as one more partition on
`drive0`. Add `,ro` to make it read-only. The name must be ASCII letters and must not
match a partition the disk already has. The guest sees an ordinary partition.

## The clipboard

`chip.getClipboard` and `chip.setClipboard` do not exist in the mod, so NeetEmu hides
them unless you pass `--clipboard`. A windowed run uses your desktop's clipboard. A
headless run keeps the text in a buffer that every machine in the run shares.

## How it differs from the mod

- The clock is the host's, so time moves during a tick.
- Each machine gets its full ticket allowance every tick; there is no server sharing
  them out.
- Peripheral and network events reach the guest at tick boundaries.
- The target is NEET 0.5.3.

Test on a real NEET computer before a release.

## Layout

| Path | Contents |
| --- | --- |
| `src/` | the emulator; `src/api/` holds the host tables the guest sees |
| `csrc/` | the SDL3 window shim, compiled by `build.rs` |
| `tests/` | integration tests; each one builds its own disk |
| `docs/` | the exec protocol |
| `third_party/Neet-YSLua` | the interpreter, as a submodule |

## License

MIT; see [`LICENSE`](LICENSE). YSLua carries its own license in `third_party/Neet-YSLua`.
