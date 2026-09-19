# Lunistice port parity

The authoritative source for this port is
`C:\Projekte\lunistice-auto-splitter\src\lib.rs`. The SplitScript port is
[`examples/lunistice.split`](../examples/lunistice.split).

## Feature mapping

| Rust autosplitter behavior | SplitScript implementation |
| --- | --- |
| Attach to `Lunistice.exe`, then `Lunistice-Demo.exe` | Ordered process list in `state` |
| Cancel initialization when the process closes | Generated process-lifetime attach continuation |
| Unity IL2CPP measured 2022.3 registration discovery | `state Unity.il2cpp(Il2CppProfile.unity2022_3_0f1X64())` prepares the generated schema binder |
| `Assembly-CSharp`, `GameManager`, and `Timer` lookup | Top-level `image` and `class` declarations |
| Original/DLC GameManager binding race | Attachment-wide enum conditions and explicit `from` names |
| C# property backing fields | Transparent `<Name>k__BackingField` matching |
| `Instance` / legacy `_instance` singleton race | `static Timer instance from ["Instance", "_instance"]` |
| Derived IL2CPP class field bindings | Generated live references consumed by expression-backed state fields |
| `Watcher<GameManager>`, `Watcher<Timer>` | GC `current`/`old` snapshots; failed fields retain their last accepted values |
| Adjacent minutes, seconds, and hundredths reads | One naturally laid-out `LevelTimeParts` struct read and local GC deserialization |
| DLC managed scene name | Schema-declared `String scene maxLength 16` with bounded surrogate-aware decoding |
| Points, resets, level time, level/scene, character runtime variables | GC string formatting plus `setVariable` |
| 1 Hz detached polling and 120 Hz attached polling | Language defaults apply 120 Hz before cooperative `onAttach` discovery and restore 1 Hz on process close |
| Auto-start when the in-game timer begins in the first level | `timerStopped` transition plus base level / DLC Shrine checks |
| A timer start initializes accumulated run state | Bare attempt-scoped globals are initialized by timer-global `onStart` and released after reset |
| Game-time accumulation across level-clock rollovers | `runTimeSeconds` is updated and consumed inside `gameTime`, plus `Duration.fromSeconds` |
| Reset on a rollover before leaving the first level | `reset` action |
| Results and final-level-to-credits splits | `split` action with both original predicates |
| Permanently paused timer game time with explicit game time | `isLoading { return true }` plus `gameTime` |
| Base and DLC character labels | `characterName` mapping with all Rust variants |
| No user-configurable settings in the Rust splitter | No settings declared; the separate `examples/lso_desktop_settings.split` port exercises the complete live settings API |

## Verification

`tests/lunistice_runtime.mjs` constructs synthetic but structurally accurate
measured Unity 2022.3 IL2CPP assembly, image, class, field, static-table, singleton, and managed
string data. Its discovery signatures are separated across a 40 MiB module so
the test requires many cooperative scan polls and proves the attached rate is
selected before scanning starts. It executes the generated WebAssembly GC module in both base and
DLC configurations and verifies:

- base-game/DLC schema and executable fallback selection;
- inherited managed-field discovery across a game-defined base class;
- focused rejection of a mixed metadata shape that matches no declared
  conditional shape;
- deterministic missing and ambiguous class and field diagnostics;
- singleton replacement without duplicating static-root reads;
- automatic and runner-initiated starts;
- Results and credits splits;
- first-level rollback reset;
- accumulated game time after a level-time rollover;
- all runtime variable values and character/level formatting;
- 1/120/1 Hz detached, attached, and process-close tick-rate changes;
- retention of the last accepted snapshot across a deliberately failed read;
- a single 12-byte host read for the three adjacent level-time components.

Run the complete verification with:

```console
cargo test
cargo clippy --all-targets -- -D warnings
cargo run --bin splitc -- examples/lunistice.split -o target/lunistice.wasm
wasm-tools validate --features all target/lunistice.wasm
node tests/lunistice_runtime.mjs target/lunistice.wasm
node tests/lunistice_runtime.mjs target/lunistice.wasm --dlc
```

## Live demo validation

On 2026-09-19, the demo at `C:\Games\Lunistice-Demo` reported Unity
`2022.3.13f1`, with binary file version `2022.3.13.37029`. Its x64 modules
successfully attached using both the explicit `unity2022_3_0f1X64()` profile and
automatic selection, which logged `UNITY_2022_3_0F1_X86_64`. Both runs completed
in 122 accelerated host updates after the shared field-cursor migration, with
no failed process reads (30,173 explicit; 30,190 automatic), and exposed
`Scene = Title`, `Character = Hana`, points, resets, and level time. This checks
real attachment, metadata binding, snapshots, and managed-string decoding;
gameplay transitions remain covered by the synthetic timer scenarios.

With the demo already running, repeat the read-only probe using
`node scripts/probe-lunistice.mjs target/verify/lunistice.release.wasm` after
`cargo xtask conformance`. The probe uses the VS Code extension's built native
process bridge and an isolated timer host. It neither launches the game nor
changes a real LiveSplit timer. Close the demo after live checks.
