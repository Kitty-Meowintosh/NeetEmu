# NeetEmu

An emulator for NEET computers: the real YSLua interpreter, a reimplemented host API.
`README.md` covers building and running, `docs/exec.md` the `neetemu:port` device and the
exec protocol.

The emulator knows no guest operating system. Anything specific to one belongs in that
guest's own repository.

YSLua is a submodule. Interpreter fixes go on a branch of the fork it points at, never in
this tree.

`cargo test`, `cargo clippy --all-targets` and `cargo fmt --check` stay clean.

# Rules on comments

- One sentence, or none. Never two.
- No rationale, history, comparison to other systems, or promotion.
- Annotations (LuaDoc, doxygen) stay short: one line of prose per item, no restating the
  signature.
- A comment that records an upstream quirk names the file it came from and nothing else.
- Never reference `NEET-BUGS.md`; it is not published.
