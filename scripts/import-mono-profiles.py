"""Import the pinned ASR Mono layout facts into SplitScript's source library.

Usage: python scripts/import-mono-profiles.py PATH_TO_ASR_CHECKOUT [--check]
Only measured layout data and the documented fallback tables are translated;
no host ABI is copied.
"""
import json
from pathlib import Path
import re
import subprocess
import sys
import uuid

PIN = "cf732d3aeac7509c8ab5f29a4d0d28f16487d245"
ROOT = Path(__file__).resolve().parents[1]
ASR = Path(sys.argv[1]).resolve()
CHECK = sys.argv[2:] == ["--check"]
if sys.argv[2:] and not CHECK:
    raise SystemExit("Expected an ASR checkout path and optional --check")
revision = subprocess.check_output(
    ["git", "-c", f"safe.directory={ASR.as_posix()}", "-C", str(ASR), "rev-parse", "HEAD"], text=True
).strip()
if revision != PIN:
    raise SystemExit(f"Expected ASR {PIN}, found {revision}")
changes = subprocess.check_output(
    ["git", "-c", f"safe.directory={ASR.as_posix()}", "-C", str(ASR),
     "diff", "HEAD", "--", "src/game_engine/unity/mono/offsets.rs",
     "src/game_engine/unity/mono/builds.rs", "src/game_engine/unity/mono/linux_builds.rs"], text=True
)
if changes:
    raise SystemExit("The pinned Mono source files have uncommitted changes")
SOURCE = ASR / "src/game_engine/unity/mono"

# Source member, SplitScript member, optional, documentation.
FIELDS = [
    ("assembly.aname", "assemblyName", True, "Assembly-relative name pointer offset, when available."),
    ("assembly.image", "assemblyImage", False, "Offset of the assembly image pointer."),
    ("image.assembly_name", "imageAssemblyName", True, "Image-relative assembly name pointer offset, when available."),
    ("image.class_cache", "imageClassCache", False, "Offset of the image class-cache hash table."),
    ("hash_table.size", "hashSize", False, "Offset of the signed hash-table bucket count."),
    ("hash_table.table", "hashTable", False, "Offset of the bucket array pointer."),
    ("class.class_kind", "classKind", True, "Offset of the class-kind byte, when measured."),
    ("class.instance_size", "classInstanceSize", True, "Offset of the boxed instance size, when measured."),
    ("class.parent", "classParent", False, "Offset of the parent class pointer."),
    ("class.nested_in", "classDeclaring", True, "Offset of the declaring class pointer, when measured."),
    ("class.name", "className", False, "Offset of the class name pointer."),
    ("class.namespace", "classNamespace", False, "Offset of the namespace pointer."),
    ("class.vtable_size", "classVtableSize", False, "Class method-count offset, or old MonoVTable.data offset."),
    ("class.fields", "classFields", False, "Offset of the field array pointer."),
    ("class.runtime_info", "classRuntimeInfo", False, "Offset of the class runtime-information pointer."),
    ("class.field_count", "classFieldCount", False, "Offset of the signed field count."),
    ("class.next_class_cache", "classNextCache", False, "Offset of the next class in a cache bucket."),
    ("generic.generic_class", "genericClass", True, "Generic-instantiation descriptor offset, when measured."),
    ("generic.container_class", "genericContainer", True, "Generic definition pointer offset, when measured."),
    ("type_words.data", "typeData", True, "Offset of a MonoType data pointer, when measured."),
    ("type_words.kind", "typeKind", True, "Offset of a MonoType kind byte, when measured."),
    ("field.type_", "fieldType", True, "Field MonoType pointer offset, when measured."),
    ("field.name", "fieldName", False, "Offset of the field name pointer."),
    ("field.offset", "fieldValue", False, "Offset of the field value offset."),
    ("field.alignment", "fieldStride", False, "Byte stride between field metadata entries."),
    ("v_table.vtable", "vtable", False, "Offset of the modern vtable's method slots."),
]


def flatten(text):
    text = re.sub(r"//[^\n]*", "", text)
    values = {}
    for group, body in re.findall(r"(\w+):\s*\w+\s*\{([^{}]*)\}", text):
        for member, raw in re.findall(r"(\w+):\s*(None|Some\(0x[\da-fA-F]+\)|0x[\da-fA-F]+|\d+)\s*[,}]?", body):
            values[f"{group}.{member}"] = None if raw == "None" else int(raw.removeprefix("Some(").removesuffix(")"), 0)
    assert set(values) == {field[0] for field in FIELDS}, values.keys()
    return values


fallback_text = (SOURCE / "offsets.rs").read_text(encoding="utf-8")
fallbacks = []
for match in re.finditer(r"\(BinaryFormat::PE, Version::(\w+), PointerSize::Bit(32|64)\) => Some\(&Self \{(.*?)\n            \}\)", fallback_text, re.S):
    version, width, body = match.groups()
    fallbacks.append(dict(version=version, width=int(width), offsets=flatten(body)))
assert len(fallbacks) == 8

build_text = (SOURCE / "builds.rs").read_text(encoding="utf-8").split("#[cfg(test)]")[0]
builds = []
for match in re.finditer(r'Build \{\s*debug_id: debug_id\("([^"]+)", (\d+)\),\s*pointer_size: PointerSize::Bit(32|64),\s*version: Version::(\w+),\s*offsets: MonoOffsets \{(.*?)\n        \},\n    \}', build_text, re.S):
    guid, age, width, version, body = match.groups()
    builds.append(dict(guid=guid, age=int(age), width=int(width), version=version, offsets=flatten(body)))
assert len(builds) == len(re.findall(r"debug_id: debug_id\(", build_text)) and len(builds) > 20


linux_fallbacks = []
for match in re.finditer(r"\(BinaryFormat::ELF(?: \| BinaryFormat::MachO)?, Version::(\w+), PointerSize::Bit64\) => (?:\{\s*)?Some\(&Self \{(.*?)\n\s*\}\)", fallback_text, re.S):
    version, body = match.groups()
    linux_fallbacks.append(dict(version=version, width=64, offsets=flatten(body)))
assert len(linux_fallbacks) == 4
linux_text = (SOURCE / "linux_builds.rs").read_text(encoding="utf-8").split("#[cfg(")[0]
linux_layouts = {name: flatten(body) for name, body in re.findall(
    r"static (UNITY_\w+): MonoOffsets = MonoOffsets \{(.*?)\n\};", linux_text, re.S)}
linux_builds = []
for label, identity, width, version, layout in re.findall(
    r'// ([^\n]+)\n    Build \{\s*build_id: &id::<\d+>\("([^"]+)"\),\s*pointer_size: PointerSize::Bit(\d+),\s*version: Version::(\w+),\s*offsets: &(\w+),', linux_text):
    linux_builds.append(dict(label=label, build_id=identity, width=int(width), version=version, layout=layout, offsets=linux_layouts[layout]))
assert len(linux_builds) == 13 and all(row['width'] == 64 for row in linux_builds)


def value(raw, optional):
    return "None" if raw is None else f"Some({hex(raw)})" if optional else hex(raw)


def constructor(row, indent="        "):
    lines = [indent + "return MonoLayout {", indent + f"    version: MonoVersion.{row['version']},"]
    for source, name, optional, _ in FIELDS:
        lines.append(indent + f"    {name}: {value(row['offsets'][source], optional)},")
    return "\n".join(lines + [indent + "}"])


lines = ["    // BEGIN GENERATED MONO PROFILES", f"    // ASR {PIN}.",
         "    // Regenerate with scripts/import-mono-profiles.py; optional facts stay absent when unknown."]
lines += ["    private static fn forVersion(version: MonoVersion, pointerSize: PointerSize) -> MonoLayout {"]
for row in fallbacks[:-1]:
    lines += [f"        if version == MonoVersion.{row['version']} && pointerSize == PointerSize.Bit{row['width']} {{", constructor(row, "            "), "        }"]
lines += [constructor(fallbacks[-1]), "    }", "", "    private static fn forBuild(identity: PeDebugId, pointerSize: PointerSize) -> MonoLayout?! {"]
for index, row in enumerate(builds):
    raw = ", ".join(str(byte) for byte in uuid.UUID(row["guid"]).bytes_le)
    lines += [f"        if identity == PeDebugId.fromParts([{raw}], {row['age']}) {{",
              f"            if pointerSize != PointerSize.Bit{row['width']} {{ throw \"Mono build identity has the wrong pointer width\" }}",
              f"            return Ok(Some(MonoLayout.build{index}()))", "        }"]
lines += ["        return Ok(None)", "    }"]
for index, row in enumerate(builds):
    lines += ["", f"    // PDB {row['guid']}, age {row['age']}; {row['width']}-bit {row['version']}.",
              f"    private static fn build{index}() -> MonoLayout {{", constructor(row), "    }"]
lines += ["", "    private static fn forLinuxVersion(version: MonoVersion) -> MonoLayout {"]
for row in linux_fallbacks[:-1]:
    lines += [f"        if version == MonoVersion.{row['version']} {{", constructor(row, "            "), "        }"]
lines += [constructor(linux_fallbacks[-1]), "    }", "", "    private static fn forLinuxBuild(identity: [u8]) -> MonoLayout? {"]
linux_distinct = list(dict.fromkeys(row['layout'] for row in linux_builds))
for row in linux_builds:
    raw = ", ".join(str(byte) for byte in bytes.fromhex(row['build_id']))
    index = linux_distinct.index(row['layout'])
    lines += [f"        // {row['label']}", f"        if identity == [{raw}] {{ return Some(MonoLayout.linuxBuild{index}()) }}"]
lines += ["        return None", "    }"]
for index, name in enumerate(linux_distinct):
    row = next(row for row in linux_builds if row['layout'] == name)
    lines += ["", f"    private static fn linuxBuild{index}() -> MonoLayout {{", constructor(row), "    }"]
lines += ["    // END GENERATED MONO PROFILES"]
generated = "\n".join(lines)
library = ROOT / "stdlib/standard.split"
text = library.read_text(encoding="utf-8")
if "    // BEGIN GENERATED MONO PROFILES" in text:
    text = re.sub(r"    // BEGIN GENERATED MONO PROFILES.*?    // END GENERATED MONO PROFILES", lambda _: generated, text, flags=re.S)
else:
    start = text.index("    // Selects the centralized PE64 layout retained by MonoModule.")
    end = text.index("\n}\n", start)
    text = text[:start] + generated + text[end:]
    start = text.index("private struct MonoLayout {") + len("private struct MonoLayout {")
    end = text.index("    // BEGIN GENERATED MONO PROFILES", start)
    fields = ["", "    /// The runtime family associated with these measured facts.", "    private version: MonoVersion,"]
    for _, name, optional, documentation in FIELDS:
        fields += [f"    /// {documentation}", f"    private {name}: u64{'?' if optional else ''},"]
    text = text[:start] + "\n".join(fields) + "\n\n" + text[end:]
destination = ROOT / "tests/fixtures/mono-pe-profiles.json"
catalog = json.dumps(dict(asr_revision=PIN, fallbacks=fallbacks, builds=builds), indent=2) + "\n"
linux_destination = ROOT / "tests/fixtures/mono-elf-profiles.json"
linux_catalog = json.dumps(dict(asr_revision=PIN, fallbacks=linux_fallbacks, builds=linux_builds), indent=2) + "\n"
if CHECK:
    if linux_destination.read_text(encoding="utf-8") != linux_catalog:
        raise SystemExit("Generated Linux Mono profiles differ; rerun without --check")
    if library.read_text(encoding="utf-8") != text or destination.read_text(encoding="utf-8") != catalog:
        raise SystemExit("Generated Mono profiles differ; rerun without --check")
    print(f"Verified {len(builds)} measured PE profiles and {len(fallbacks)} PE fallbacks; {len(linux_builds)} ELF profiles and {len(linux_fallbacks)} ELF fallbacks")
    raise SystemExit(0)
linux_destination.write_text(linux_catalog, encoding="utf-8", newline="\n")
library.write_text(text, encoding="utf-8", newline="\n")
destination.parent.mkdir(parents=True, exist_ok=True)
destination.write_text(catalog, encoding="utf-8")
print(f"Imported {len(builds)} measured PE profiles and {len(fallbacks)} PE fallbacks; {len(linux_builds)} ELF profiles and {len(linux_fallbacks)} ELF fallbacks")
