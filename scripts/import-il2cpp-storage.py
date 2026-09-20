"""Measure supplemental IL2CPP storage facts from hash-verified player PDBs.

Usage: python scripts/import-il2cpp-storage.py MANIFEST PDB_DIRECTORY PDB_TOOL [--check]
See scripts/unity-pdb/README.md for the pinned manifest and input names.
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
MANIFEST_URL = "https://raw.githubusercontent.com/ero-qt/auto-splitting-test-fixtures/e2d5e075b253d4a9c9c2c1b535ca2d3d671ceea7/manifest.json"

MANIFEST_SHA256 = "aa93142689fc59a7575cf8ccd91345ef0b0cd1e2fcdd33f73497a3ac78f478bd"


def measure(manifest_path, cache, tool):
    manifest_bytes = manifest_path.read_bytes()
    if hashlib.sha256(manifest_bytes).hexdigest() != MANIFEST_SHA256:
        raise ValueError("Fixture manifest differs from the pinned revision")
    manifest = json.loads(manifest_bytes)["fixtures"]
    catalog = json.loads((ROOT / "tests/fixtures/il2cpp-pe-profiles.json").read_text(encoding="utf-8"))["builds"]
    output = ROOT / "tests/fixtures/il2cpp-storage-profiles.json"
    data = json.loads(output.read_text(encoding="utf-8"))
    data["provenance"] = "Value-type bits measured from hash-verified GameAssembly PDBs; 2021.3.11f1 remains source-derived. ASR offsets are cross-checked, not changed."
    data["fixture_manifest"] = {"url": MANIFEST_URL, "sha256": hashlib.sha256(manifest_bytes).hexdigest()}
    measurements = {}
    for profile in catalog:
        parts = profile["name"].split("_")
        version = ".".join(parts[1:-2] if profile["width"] == 64 else parts[1:-1]).lower()
        variant = f"win-x{64 if profile['width'] == 64 else 86}-il2cpp"
        fixture = next((f for f in manifest if f["engine"] == "unity" and f["version"] == version and f["variant"] == variant), None)
        if fixture is None:
            if version != "2021.3.11f1":
                raise ValueError(f"Missing fixture: {version} {variant}")
            continue
        file = next(f for f in fixture["files"] if f["path"].endswith("/GameAssembly.pdb"))
        pdb = cache / f"{version}-{variant}.pdb"
        with pdb.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        if digest != file["sha256"]:
            raise ValueError(f"PDB hash mismatch: {pdb}")
        layout = json.loads(subprocess.check_output([str(tool), str(pdb)]))
        cls = layout["Il2CppClass"]["fields"]
        typ = layout["Il2CppType"]["fields"]
        generic = layout["Il2CppGenericClass"]["fields"]
        expected = profile["offsets"]
        for name, member in [("name", "name"), ("namespace", "namespaze"),
                             ("declaring_type", "declaringType"), ("parent", "parent"),
                             ("fields", "fields"), ("static_fields", "static_fields"),
                             ("instance_size", "instance_size"), ("field_count", "field_count")]:
            if cls[member]["offset"] != expected[f"class.{name}"]:
                raise ValueError(f"{profile['name']}: ASR class.{name} does not match its PDB")
        if generic["cached_class"]["offset"] != expected["generic.cached_class"]:
            raise ValueError(f"{profile['name']}: cached_class mismatch")
        if typ["data"]["offset"] != expected["type_.data"] or typ["type"]["bits"] != 8 or typ["type"]["bit"] != 16 or typ["type"]["offset"] + 2 != expected["type_.kind"]:
            raise ValueError(f"{profile['name']}: Il2CppType layout mismatch")
        if "valuetype" in cls:
            flag = cls["valuetype"]
            location = {"owner": "Il2CppClass", **flag}
            bit = flag["offset"] * 8 + flag["bit"]
        else:
            if cls["byval_arg"].get("type") != "Il2CppType":
                raise ValueError("byval_arg is not an embedded Il2CppType")
            flag = typ["valuetype"]
            location = {"owner": "Il2CppType", "class_offset": cls["byval_arg"]["offset"], **flag}
            bit = (location["class_offset"] + flag["offset"]) * 8 + flag["bit"]
        if flag["bits"] != 1:
            raise ValueError("Value-type flag is not one bit")
        previous = data["profiles"].get(profile["name"])
        if previous is not None and previous != bit:
            raise ValueError(f"{profile['name']}: prior audited bit {previous} differs from PDB bit {bit}")
        data["profiles"][profile["name"]] = bit
        measurements[profile["name"]] = {
            "asset": fixture["asset"], "archive_sha256": fixture["sha256"],
            "pdb_path": file["path"], "pdb_sha256": digest,
            "value_type_flag": location,
            "class_instance_size": cls["instance_size"]["offset"],
            "generic_cached_class": generic["cached_class"]["offset"],
        }
    if len(measurements) != 20 or set(data["profiles"]) != {p["name"] for p in catalog}:
        raise ValueError("Expected twenty measured PDBs and twenty-two proven profiles")
    data["measurements"] = measurements
    text = json.dumps(data, indent=2) + "\n"
    if "--check" in sys.argv:
        if output.read_text(encoding="utf-8") != text:
            raise ValueError("Checked-in IL2CPP storage facts differ from measurements")
    else:
        output.write_text(text, encoding="utf-8", newline="\n")
    print("Verified 20 PDBs, 220 ASR layout facts, and all 22 value-type bits")


if __name__ == "__main__":
    measure(Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3]).resolve())
