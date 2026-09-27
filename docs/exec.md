# The exec protocol

`neetemu exec` runs one statement inside a guest and exits with the guest's status. The
emulator has no idea what a shell is: it attaches a `neetemu:port` device to the machine,
and a program in the guest claims the port and answers the frames below. Any operating
system that implements this protocol can be driven by `neetemu exec`.

## The device

`neetemu:port` exists only in NeetEmu. The guest finds it the same way as any other
peripheral, through `io.getPeripherals()` and `io.wrapPeripheral(id)`.

| Method | Contract |
| --- | --- |
| `read(n)` | up to `n` bytes from the host, at most 16 KiB, or `""` when nothing is waiting |
| `write(data)` | the number of bytes accepted, which falls short when 64 KiB are already unread and is 0 once the host has gone |
| `poll()` | how many bytes are waiting, and whether the host still holds its end |
| `close()` | drops the guest's end; later calls raise `port is closed` |

No call blocks. When bytes arrive, the device raises a `neetemu:port` event on the
`Peripheral` queue with the arguments `uuid, "data"`. Only one such event is
outstanding at a time: the next one is raised after the guest calls `read`. The
arguments `uuid, "closed"` mean the host has let go. A guest can park on `chip.sleep()`
until one of these events arrives, so an idle port costs nothing.

`--peripheral neetemu:port` attaches a port to an ordinary run, for developing the
guest side without `exec`.

## Frames

Both directions carry the same frame format: one opcode byte, a four-byte big-endian
payload length, then the payload. A payload may be up to 1 MiB.

| Opcode | Direction | Payload |
| --- | --- | --- |
| `R` | guest → host | the guest has claimed the port; free text, such as the features it offers |
| `X` | host → guest | a statement to run |
| `I` | host → guest | bytes for the statement's standard input |
| `Z` | host → guest | standard input has ended |
| `O` | guest → host | bytes from standard output |
| `E` | guest → host | bytes from standard error |
| `S` | guest → host | the statement has finished; its exit status in decimal |
| `!` | guest → host | the guest could not do what was asked; a message |

## A session

1. The host boots the machine with the port attached and waits for `R`.
2. The host sends `X`, then forwards its own standard input as `I` frames and ends
   with `Z`.
3. The guest sends `O` and `E` as the statement produces output, and `S` once the
   statement has finished and all of its output has been sent.
4. The host writes `O` and `E` payloads to its own standard output and standard error
   unchanged, then exits with the status from `S`, clamped to 0–255.
5. The host closes its end. The guest sees `closed`, or `poll()` returning `false`,
   and lets go of the port.

A `!` frame at any point ends the run with an error. The emulator does not interpret
output bytes, so a guest that wants colour on the host terminal writes ANSI sequences
and UTF-8 itself.

`tests/exec.rs` contains a complete guest in about sixty lines of Lua.
