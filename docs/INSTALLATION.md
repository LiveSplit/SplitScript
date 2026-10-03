# Installing SplitScript

Choose the Visual Studio Code extension unless an editor or automated workflow
specifically needs native command-line tools. Both paths use the same compiler,
formatter, diagnostics, documentation catalog, and language service.

| Path | Includes | Best for |
| --- | --- | --- |
| VS Code VSIX | Editor support, embedded compiler and language server, documentation, build commands, and the desktop autosplitter debugger | Writing and testing autosplitters without installing a compiler |
| Native `splitc` and `splitls` | Command-line compilation, formatting, documentation, watch builds, and a standard-input/output language server | Other editors, scripts, and build automation |

SplitScript is still an early moving language. The `latest` rolling release
follows each verified `master` build, and source compatibility can change.

## Visual Studio Code extension

1. Open the [SplitScript Marketplace page][marketplace] in VS Code and choose
   **Install Pre-Release Version**.
2. Open a folder, save a file with the `.split` extension, and run
   **SplitScript: Open Documentation**.

VS Code keeps pre-release users on the pre-release update channel. For a rolling
build from the latest verified `master` commit instead, download
`splitscript-latest.vsix` from the [latest SplitScript release][latest-release]
and run **Extensions: Install from VSIX**.

The VSIX is batteries-included: do not install `splitc`, `splitls`, Rust, Node,
or a WebAssembly toolchain merely to use the extension. Release packaging
checks artifact integrity and reports the compiler Wasm and compressed VSIX
sizes without enforcing fixed size budgets. The package contains native
debugger bridges for Windows x64, Linux x64
and ARM64, and macOS Intel and Apple Silicon; only the bridge for the running
desktop host is loaded.

The language and build services run in separate workers with independent
embedded compiler instances. A long build therefore does not replace the
language server, although both compiler instances contribute to the extension's
memory use. The autosplitter runtime starts in another worker only while a
debug session is active. Runtime statistics retain a bounded 2,048-tick window,
and sidebar updates are coalesced to at most five per second. No fixed peak-RAM
guarantee has been established yet; report a reproducible source file and the
operation being performed if memory does not recover after a build, language
server restart, or stopped debug session.

### Supported extension hosts

- Desktop VS Code 1.125 or newer provides language tooling, builds, and the
  autosplitter debugger.
- Browser, remote, virtual, and untrusted workspaces retain language tooling and
  builds through the VS Code workspace filesystem.
- Running an autosplitter currently requires a trusted local desktop workspace.
- Native process debugging supports Windows x64, Linux x64 and ARM64, and macOS
  Intel and Apple Silicon. macOS process attachment remains subject to
  `task_for_pid` authorization and target code-signing policy.

Generated autosplitters are WebAssembly GC modules for the Auto Splitting
Runtime ABI. The extension creates the module but does not install it into a
timer. Load the neighboring `.wasm` file through the autosplitting host's normal
local-module workflow; hosts without WebAssembly GC support cannot instantiate
it.

### Builds, output, and recovery

**SplitScript: Build Release** saves the active script and atomically replaces
a neighboring release module: `game.split` produces `game.wasm`.
**SplitScript: Start Debug Watch** writes the same neighboring path with the
debug profile and rebuilds after later saves. Build diagnostics and progress
appear in the **SplitScript Compiler** Output channel. A failed, cancelled, or
superseded build leaves the previous successful module intact and removes its
temporary output.

If every language feature becomes unresponsive together, run **SplitScript:
Restart Language Server**. If a debug session fails, inspect **Auto Splitting
Runtime** in the Output panel, stop the session, and start it again after fixing
the reported source or host error. A stopped compiler worker rejects its pending
requests; a later command creates a fresh worker rather than reusing failed
state.

## Native command-line tools

The [latest SplitScript release][latest-release] provides one archive containing
both `splitc` and `splitls` for each supported native host:

| Host | Release asset |
| --- | --- |
| Windows x64 | [`splitscript-windows-x64.zip`][windows-x64] |
| Linux x64 | [`splitscript-linux-x64.tar.gz`][linux-x64] |
| Linux ARM64 | [`splitscript-linux-arm64.tar.gz`][linux-arm64] |
| macOS Intel | [`splitscript-macos-x64.tar.gz`][macos-x64] |
| macOS Apple Silicon | [`splitscript-macos-arm64.tar.gz`][macos-arm64] |

Download and extract the matching archive. The created platform-named directory
contains both executables and a copy of this installation guide. Keep the
directory together, add it to `PATH`, or configure an editor with the absolute
path to `splitls`. Windows executable names end in `.exe`.

These are rolling early builds from the commit named by the `latest` release,
not stable versioned releases. Each runner starts `splitc --version` and
`splitls` before publishing its archive. `SHA256SUMS` on the release records the
digest of every native archive and the VSIX.

To build the tools yourself instead, use a repository checkout with the latest
stable Rust toolchain:

```console
cargo build --profile max-opt --bin splitc --bin splitls
```

The executables are written to `target/max-opt` (`.exe` is added on Windows).
Keep them there, copy them to a directory already on `PATH`, or configure an
editor with their absolute paths. The `max-opt` profile is the distribution
profile; it favors compiler execution speed and executable size at the cost of
a slower initial build.

The complete repository verification matrix runs on Windows. Before packaging,
each of the five native release runners also runs `cargo xtask conformance`: all
compiler tests plus every compiled and validated runtime fixture execute on
Windows x64, Linux x64/ARM64, and macOS x64/ARM64. The packaged CLI and language
server are the same revision-stamped max-opt binaries exercised by that job.
This is separate from the five-platform debugger bridges inside the VSIX.

Compile, watch, format, or browse documentation with:

```console
splitc game.split -o game.wasm --profile release
splitc watch game.split -o game.wasm
splitc fmt game.split
splitc docs
```

When running a source build without adding `target/max-opt` to `PATH`, replace
`splitc` above with `target/max-opt/splitc` or
`target\max-opt\splitc.exe`. `splitc watch` performs an initial build and then
rebuilds after source changes. Compilation failures keep the last successful
output. `splitc fmt` follows applicable `.editorconfig` formatting properties.
Run `splitc --help` or a subcommand's `--help` for the current command surface.

`splitls` is a Language Server Protocol server over standard input and output.
For another editor, configure the server command as the absolute path to
`splitls` with no arguments and associate it with `.split` files.
Do not configure `splitls` as a TCP server, and do not parse its standard output
as logs: that stream carries framed LSP messages.

Native CLI builds produce the same WebAssembly GC and Auto Splitting Runtime ABI
as extension builds. They likewise do not install, register, or execute the
result in a timer host.

[latest-release]: https://github.com/LiveSplit/SplitScript/releases/tag/latest
[marketplace]: https://marketplace.visualstudio.com/items?itemName=LiveSplit.splitscript
[windows-x64]: https://github.com/LiveSplit/SplitScript/releases/download/latest/splitscript-windows-x64.zip
[linux-x64]: https://github.com/LiveSplit/SplitScript/releases/download/latest/splitscript-linux-x64.tar.gz
[linux-arm64]: https://github.com/LiveSplit/SplitScript/releases/download/latest/splitscript-linux-arm64.tar.gz
[macos-x64]: https://github.com/LiveSplit/SplitScript/releases/download/latest/splitscript-macos-x64.tar.gz
[macos-arm64]: https://github.com/LiveSplit/SplitScript/releases/download/latest/splitscript-macos-arm64.tar.gz
