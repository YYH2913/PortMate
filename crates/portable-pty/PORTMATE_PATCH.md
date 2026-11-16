# PortMate's portable-pty 0.9.0 patch

Upstream: Wez Furlong's [wezterm/pty](https://github.com/wezterm/wezterm/tree/f8921727a11b9f8b073e8c24821d72fd41283500/pty), MIT license (unchanged `LICENSE.md`).
Crate provenance and unchanged-source hashes: `PORTMATE_UPSTREAM.json`.

PortMate opens a new blank terminal, so its Windows ConPTY must **not** request
`PSEUDOCONSOLE_INHERIT_CURSOR`. That flag waits for a cursor-position response
from an attached renderer; background/MCP-only sessions have no renderer to
answer, so startup and early commands can stall. Keep the upstream resize quirk
and Win32 input mode. Unix PTY behavior is unchanged.

Reference: https://learn.microsoft.com/windows/console/createpseudoconsole

Upstream command-builder tests are preserved under `test/rust/portable-pty/`,
with a source-only test module hook. Examples are omitted from the local package
manifest, along with example-only development dependencies. The existing real Windows cmd.exe MCP test deliberately runs without
a renderer and verifies both lines of `run_command` and `run_local_command`.
The MIT notice is included in installed and portable PortMate packages.
