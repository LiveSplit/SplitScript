# Measuring IL2CPP storage facts

The utility reads CodeView structure members and bitfields from `GameAssembly.pdb`.
It uses the `pdb` crate, accepts repeated identical definitions, and rejects
conflicting definitions. It does not run any fixture executable.

Build from the repository root:

```powershell
cargo build --release --manifest-path scripts/unity-pdb/Cargo.toml --target-dir target/unity-pdb
```

Use the [pinned fixture manifest](https://raw.githubusercontent.com/ero-qt/auto-splitting-test-fixtures/e2d5e075b253d4a9c9c2c1b535ca2d3d671ceea7/manifest.json).
Download the twenty Windows IL2CPP variants matching the named profiles, extract
only their `GameAssembly.pdb` files, and name each `<version>-<variant>.pdb`, for
example `6000.7.0a3-win-x64-il2cpp.pdb`. Keep this cache outside the build directory;
the PDBs use roughly 1.7 GB uncompressed. The manifest does not contain
2021.3.11f1, whose two profiles retain their separately recorded source evidence.

Measure and check the supplemental facts:

```powershell
python scripts/import-il2cpp-storage.py PATH_TO_MANIFEST PATH_TO_PDB_CACHE target/unity-pdb/release/unity-pdb-layouts.exe --check
```

Omit `--check` to update `tests/fixtures/il2cpp-storage-profiles.json`, then run the
ordinary IL2CPP profile importer. The storage importer verifies each PDB's SHA-256
against the manifest and checks the existing ASR class, generic, and type offsets
before deriving any bit. It also rejects disagreements with previously audited
bits and checks the relative locations used to resolve ordinary type classes
(`byval_arg` after `namespaze`, and the definition member two pointer words after
`parent`). No ASR checkout is changed.

For inspection of one PDB:

```powershell
target/unity-pdb/release/unity-pdb-layouts.exe PATH_TO_GAMEASSEMBLY_PDB
```
