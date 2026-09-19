import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { SplitScriptHost } from "./support/splitscript_host.mjs";
import { createMonoPeFixture } from "./support/mono_pe_fixture.mjs";

const [wasmPath, family] = process.argv.slice(2);
if (!wasmPath) throw new Error("usage: node tests/mono_profiles_runtime.mjs <module.wasm>");
const profiles = JSON.parse(await readFile(new URL("./fixtures/mono-pe-profiles.json", import.meta.url)));
let cases = 0;
async function run(label, fixture, expected = "exact PDB build profile") {
    const host = await SplitScriptHost.instantiate(wasmPath);
    host.addProcess("game.exe", fixture.process);
    host.start();
    host.updateUntil(() => host.messages.includes("42"), label, 2000);
    if (expected !== null) assert(host.messages.some(message => message.includes(expected)), `${label}: wrong selection ${host.messages}`);
    assert(fixture.reads < 1000, `${label}: excessive metadata work`);
    cases++;
}
if (family) {
    for (const width of [32, 64]) {
        await run(`explicit ${family} ${width}`, createMonoPeFixture({ version: family, width, exact: false }), null);
    }
} else {
    for (const profile of profiles.builds) {
        await run(profile.guid, createMonoPeFixture(profile));
    }
    for (const width of [32, 64]) {
        const profile = profiles.builds.find(p => p.width === width && p.version === "V1");
        await run(`false signature ${width}`, createMonoPeFixture({ ...profile, falseCandidate: true }));
        await run(`alternate signature ${width}`, createMonoPeFixture({ ...profile, signatureVariant: 1 }));
        await run(`module edge ${width}`, createMonoPeFixture({ ...profile, exportOffset: 0xff0 }));
        for (const version of ["V1", "V1Cattrs"]) {
            await run(`old fallback ${version} ${width}`, createMonoPeFixture({ version, width, exact: false }), "no PDB identity");
        }
        await run(`wrong age ${width}`, createMonoPeFixture({ ...profile, age: profile.age + 1, exact: false }), "unknown PDB build");
        for (const version of ["V2", "V3"]) {
            const modern = profiles.builds.find(p => p.width === width && p.version === version);
            const playerVersion = version === "V2" ? [2020, 1] : [2021, 2];
            await run(`modern missing ID ${version} ${width}`, createMonoPeFixture({ version, width, playerVersion, exact: false }), "no PDB identity");
            await run(`modern wrong age ${version} ${width}`, createMonoPeFixture({ ...modern, age: modern.age + 1, playerVersion, exact: false }), "unknown PDB build");
            const malformed = createMonoPeFixture({ ...modern, playerVersion, exact: false });
            new DataView(malformed.image.buffer).setUint32(0x810, 23, true);
            await run(`malformed ID ${version} ${width}`, malformed, "unreadable PDB identity");
            await run(`exact beats player ${version} ${width}`, createMonoPeFixture({ ...modern, playerVersion: version === "V2" ? [2021, 2] : [2020, 1] }));
        }

        const wrongWidth = createMonoPeFixture({ ...profile, width: width === 32 ? 64 : 32 });
        const host = await SplitScriptHost.instantiate(wasmPath);
        host.addProcess("game.exe", wrongWidth.process);
        host.start();
        host.updateUntil(() => host.messages.includes("Mono build identity has the wrong pointer width"), "width mismatch");
        host.update(20);
        assert(!host.messages.includes("42"), "a mismatched profile must not publish state");
        host.setProcessOpen("game.exe", false);
        host.update();
        const replacement = createMonoPeFixture(profile);
        host.addProcess("game.exe", replacement.process);
        host.updateUntil(() => host.messages.includes("42"), "retry after rejected process exits", 2000);
        cases++;

        for (const failure of ["missing instruction", "truncated instruction", "outside export"]) {
            const fixture = createMonoPeFixture(profile);
            fixture.image.fill(0, 0x500, 0x600);
            const view = new DataView(fixture.image.buffer);
            if (failure === "truncated instruction") {
                view.setUint32(0x300, 0xffd, true);
                fixture.image.set(width === 64 ? [0x48, 0x8b, 0x0d] : [0xff, 0x35], 0xffd);
            } else if (failure === "outside export") {
                view.setUint32(0x300, 0x2000, true);
            }
            const rejected = await SplitScriptHost.instantiate(wasmPath);
            rejected.addProcess("game.exe", fixture.process);
            rejected.start();
            if (failure === "outside export") {
                // The PE reader rejects the RVA before Mono sees it, so discovery
                // remains pending until module metadata changes or the process exits.
                rejected.update(30);
                assert(!rejected.messages.includes("42"));
                rejected.setProcessOpen("game.exe", false);
                rejected.update();
                rejected.addProcess("game.exe", createMonoPeFixture(profile).process);
                rejected.updateUntil(() => rejected.messages.includes("42"), "cancel invalid export", 2000);
                cases++;
                continue;
            }
            rejected.updateUntil(() => rejected.messages.some(message => message.includes(
                "instruction was not found"
            )), failure, 100);
            const reads = fixture.reads;
            rejected.update(20);
            assert(!rejected.messages.includes("42"), `${failure}: invalid discovery published state`);
            assert.equal(fixture.reads, reads, `${failure}: failed discovery kept scanning`);
            cases++;
        }
    }
}
console.log(JSON.stringify({ monoProfileCases: cases }));
