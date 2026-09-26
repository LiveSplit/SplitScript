# SplitScript for Visual Studio Code

Write statically typed autosplitters with diagnostics, completion, navigation,
documentation, formatting, and one-click WebAssembly builds. The extension
contains the SplitScript compiler and language server, so it does not require a
separate native executable.

## Start a script

1. Install the [SplitScript Marketplace pre-release][marketplace]. To test the
   newest verified `master` build instead, download the [latest SplitScript
   VSIX][latest-vsix] and install it with **Extensions: Install from VSIX**.
   These early packages may contain breaking language changes.
2. Open a folder and create a saved file ending in `.split`.
3. Run **SplitScript: Open Documentation** and open **Getting started**. That
   compiler-checked guide introduces process attachment, a typed setting, one
   memory field, `old` / `current` snapshots, and the first timer decision.
4. On desktop, use **Auto Splitter Debugger: Debug Active File** to compile and
   run a `.split` source or directly launch a `.wasm` module inside VS Code. The
   debugger hot reloads saved `.split` sources and rebuilt `.wasm` modules by
   default, including changes written by tools outside VS Code. The standard
   debug toolbar pauses and resumes the autosplitter update loop. The dedicated
   **Auto Splitter Debugger** sidebar shows
   the simulated timer, runtime statistics, user settings, the raw settings map,
   timer variables, and controls; runtime output is sent to the
   **Auto Splitting Runtime** Output channel and Debug Console.
5. Use **SplitScript: Start Debug Watch** when you only want to continuously
   rebuild a `.wasm` file without launching it.
6. Use **SplitScript: Build Release** when the module is ready to distribute.

Both build commands create a `.wasm` file beside the source file. For example,
`game.split` produces `game.wasm`. A failed or superseded build leaves the last
successful module intact.

SplitScript generates a module for the Auto Splitting Runtime ABI. Selecting or
installing that module is handled by the autosplitting host, not this extension.
Use the host's normal local-Wasm workflow to load the generated file.

## Commands

| Command | Result |
| --- | --- |
| **SplitScript: Open Documentation** | Opens the compiler-owned reference index beside the active script. |
| **SplitScript: Open Documentation for Current Symbol** | Opens the exact language or standard-library page for the symbol at the caret. Assign it a shortcut through Keyboard Shortcuts for quick reference navigation. |
| **SplitScript: Search Documentation** | Searches symbols, concepts, signatures, summaries, and migration terms. |
| **SplitScript: Start Debug Watch** | Saves and builds the active script with the debug profile, then rebuilds it after later saves. |
| **SplitScript: Stop Debug Watch** | Stops the watcher shown in the status bar. |
| **Auto Splitter Debugger: Debug Active File** | Compiles an active `.split` source or directly runs an active `.wasm` module in an isolated Node WebAssembly worker, hot reloading either input by default. If neither is active, prompts for one. |
| **Auto Splitter Debugger: Restart** | Recompiles or rereads and replaces the running WebAssembly instance. |
| **Auto Splitter Debugger: Start/Reset Timer** | Controls the debugger's simulated timer. |
| **Auto Splitter Debugger: Clear Settings Map** | Removes all values currently overridden in the runtime settings map. |
| **Auto Splitter Debugger: Reset Statistics** | Clears collected tick timings while leaving the auto splitter running. |
| **Auto Splitter Debugger: Open WebAssembly Memory** | Opens linear memory through lazily loaded Debug Adapter Protocol pages in VS Code's Hex Editor. |
| **Auto Splitter Debugger: Open Process Memory** | Selects a starting mapping and opens a process-wide lazy memory view in the Hex Editor. |
| **Auto Splitter Debugger: Show Logs** | Opens the runtime Output channel. |
| **SplitScript: Build Release** | Saves and performs one optimized build of the active script. |
| **SplitScript: Restart Language Server** | Replaces the language-service worker without reloading the editor window. |

Documentation and build commands are available from the Command Palette. The
documentation and release-build actions also appear in the `.split` editor
title; symbol documentation and build/watch actions are in the editor context
menu.

## Language support

The extension provides:

- diagnostics with related source locations and quick fixes;
- context-sensitive completion and signature help;
- hover, inferred-type hints, semantic highlighting, and document symbols;
- go to definition, references, rename, and selection ranges;
- whole-document formatting, including Format on Save;
- compiler-owned language, lifecycle, migration, and standard-library pages.

Formatting follows ancestor `.editorconfig` files by default. SplitScript uses
`indent_style`, `indent_size` / `tab_width`, `max_line_length`, `end_of_line`,
and `insert_final_newline`. The **SplitScript › Formatting** settings can
override each choice for the editor without requiring an `.editorconfig`; an
unset setting continues to inherit the file, editor, or formatter default.

The language server and build worker use separate compiler instances. A long or
failed build therefore does not replace the language server. Source and output
files are accessed through the VS Code workspace filesystem, including in
virtual workspaces.

## Requirements and current limits

- VS Code 1.125 or newer is required.
- The extension supports desktop and browser extension hosts.
- Running an autosplitter is currently a desktop-only capability and requires a
  trusted local workspace. Browser, virtual, and untrusted workspaces retain
  the compiler and language tooling.
- The debugger implements the complete timer, runtime, process, user-settings,
  and settings map/list/value ASR APIs. Native process attachment,
  liveness, module lookup, mapped ranges, and read-only memory access run in a
  Rust N-API bridge; attached processes appear in **Processes**.
  Each open process has an inline memory action that queries its readable mapped
  ranges on demand and opens a process-wide lazy view at the selected range's base
  without copying the entire process.
  Release VSIX packages include native bridges for Windows x64, Linux x64 and
  ARM64, and macOS Intel and Apple Silicon. Autosplitter ticks run at the
  requested rate independently of sidebar snapshots, which are
  coalesced to at most five updates per second. The **Statistics** panel keeps
  a bounded window of 2,048 tick timings and provides reset and lazy Wasm-memory
  actions without stopping the runtime.
  On macOS, attaching to another process uses `task_for_pid` and is subject to
  the operating system's debugger authorization and target code-signing rules.
  A denied attachment reports that permission boundary explicitly.
- WASI snapshot preview1 (WASI 0.1) is available with a read-only filesystem
  below `/mnt`. Arguments and environment variables are deliberately empty,
  and filesystem-mutating operations return `NOTCAPABLE`.
- Pause and Continue suspend and resume the autosplitter scheduler between
  entry-point calls. Pausing keeps the WebAssembly instance and debugger state
  alive, and hot reload preserves the suspended state. It cannot interrupt an
  `update` call that is already executing. Source breakpoints, stepping, and
  pausing at a source location are not implemented yet; those require the
  planned debugger-instrumented compiler mode.
- A directly launched `.wasm` file is instantiated as-is. Modules exporting
  `update` use the recurring auto-splitting loop; otherwise `_initialize` and
  `_start` are invoked once when present. A module with no conventional entry
  point still runs its WebAssembly start section during instantiation.
- Building requires a saved `.split` resource and write access beside it.
- Generated modules require an autosplitting host with WebAssembly GC enabled.
  Older host engines that disable WebAssembly GC cannot instantiate them.
- The debugger uses a simulated timer and does not control a running LiveSplit
  instance.
- Version 0.1.x is distributed through the Visual Studio Marketplace's opt-in
  pre-release channel.

## Troubleshooting

If a build fails, open **SplitScript Compiler** in the Output panel and inspect
the source diagnostics in **Problems**. Fixing the source and saving it is
enough to retry an active debug watch.

If hover, completion, highlighting, and navigation all stop together, run
**SplitScript: Restart Language Server**. If the problem is reproducible, keep
the smallest source that triggers it and the language-server output so it can be
reported without losing the failing state.

If a build targets the wrong file, make sure the intended `.split` editor is
active when starting the command. Untitled files prompt for a save location;
the resulting saved document is the one compiled.

[latest-vsix]: https://github.com/LiveSplit/SplitScript/releases/download/latest/splitscript-latest.vsix
[marketplace]: https://marketplace.visualstudio.com/items?itemName=LiveSplit.splitscript
