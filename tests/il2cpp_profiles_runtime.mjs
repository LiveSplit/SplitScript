import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { SplitScriptHost } from "./support/splitscript_host.mjs";
import { createIl2cppPeFixture } from "./support/il2cpp_pe_fixture.mjs";

const [wasmPath, explicitWidth] = process.argv.slice(2);
if (!wasmPath) throw new Error("usage: node tests/il2cpp_profiles_runtime.mjs <module.wasm>");
const profiles = JSON.parse(await readFile(new URL("./fixtures/il2cpp-pe-profiles.json", import.meta.url)));
let cases = 0;
async function run(label, fixture, expected) {
    const host = await SplitScriptHost.instantiate(wasmPath);
    host.addProcess("game.exe", fixture.process);
    host.start();
    host.updateUntil(() => host.messages.includes("42"), label, 2000);
    if (expected) assert(host.messages.some(message => message.includes(expected)), `${label}: wrong selection ${host.messages}`);
    assert(fixture.reads < 1000, `${label}: excessive metadata work`);
    cases++;
}
if (explicitWidth === "invalid") {
    const fixture = createIl2cppPeFixture({ version: [2022, 3, 0, 37029], playerVersion: null });
    const host = await SplitScriptHost.instantiate(wasmPath);
    host.addProcess("game.exe", fixture.process);
    host.start();
    host.updateUntil(() => host.messages.includes("IL2CPP profile is invalid or has the wrong pointer width"), "reject invalid custom profile");
    host.update(20);
    assert(!host.messages.includes("42"));
    assert(fixture.reads < 10, "invalid profile started scanning metadata");
    cases++;
} else if (explicitWidth) {
    const width = Number(explicitWidth);
    const profile = profiles.builds.find(p => p.width === width && p.version[0] === 2022);
    await run(`explicit without UnityPlayer ${width}`, createIl2cppPeFixture({ ...profile, playerVersion: null }));
    const host = await SplitScriptHost.instantiate(wasmPath);
    host.addProcess("game.exe", createIl2cppPeFixture({ ...profile, width: width === 32 ? 64 : 32 }).process);
    host.start();
    host.updateUntil(() => host.messages.includes("IL2CPP profile is invalid or has the wrong pointer width"), "reject mismatched width");
    host.update(20);
    assert(!host.messages.includes("42"));
    host.setProcessOpen("game.exe", false);
    host.update();
    host.addProcess("game.exe", createIl2cppPeFixture({ ...profile, playerVersion: null }).process);
    host.updateUntil(() => host.messages.includes("42"), "new process after width mismatch", 2000);
    cases++;
} else {
for (const profile of profiles.builds) {
    await run(profile.name, createIl2cppPeFixture(profile), profile.name);
}
for (const width of [32, 64]) {
    const profile = profiles.builds.find(p => p.width === width && p.version[0] === 2022);
    await run(`false vector ${width}`, createIl2cppPeFixture({ ...profile, falseCandidate: true }), profile.name);
    await run(`Lunistice demo nearest ${width}`, createIl2cppPeFixture({ ...profile, playerVersion: [2022, 3, 13, 37029] }), profile.name);
    for (const playerVersion of [[2023, 1, 1, 0], [2023, 1, 0, 37029]]) {
        const newest = profiles.builds.find(p => p.width === width && p.version[0] === 2023 && p.version[2] === 22);
        await run(`nearest selects newest in family ${width}`, createIl2cppPeFixture({ ...newest, playerVersion }), newest.name);
    }
    const oldest = profiles.builds.find(p => p.width === width && p.version[0] === 2018);
    await run(`older than measured ${width}`, createIl2cppPeFixture({ ...oldest, playerVersion: [5, 6, 0, 0] }), oldest.name);
    const newest = profiles.builds.find(p => p.width === width && p.version[0] === 6000 && p.version[1] === 7);
    await run(`newer than measured ${width}`, createIl2cppPeFixture({ ...newest, playerVersion: [6001, 1, 0, 0] }), newest.name);
    if (width === 32) {
        for (const storeVariant of [1, 2]) {
            await run(`x86 store ${storeVariant}`, createIl2cppPeFixture({ ...profile, storeVariant }), profile.name);
        }
        const fixture = createIl2cppPeFixture({ ...profile, storeVariant: 2 });
        fixture.image.set([0xc1, 0xea, 2, 0x52, 0xe8, 0, 0, 0, 0, 0xa3], 0x780);
        new DataView(fixture.image.buffer).setUint32(0x78a, Number(fixture.base + 0xc24n), true);
        await run("earliest x86 store wins across patterns", fixture, profile.name);
    }
    for (const failure of ["missing vector", "truncated vector", "truncated vector global", "outside table", "truncated store", "absent name"]) {
        const fixture = createIl2cppPeFixture(profile);
        const data = new DataView(fixture.image.buffer);
        if (failure === "missing vector") fixture.image.fill(0, 0x300, 0x310);
        if (failure === "truncated vector") {
            fixture.image.copyWithin(0x1ff8, 0x300, 0x308);
            fixture.image.fill(0, 0x300, 0x310);
        }
        if (failure === "truncated vector global") {
            const begin = fixture.base + 0x2000n - BigInt(width / 8);
            if (width === 64) {
                data.setInt32(0x305, Number(begin - fixture.base) - 0x309, true);
                data.setInt32(0x30c, 0x2000 - 0x310, true);
            } else {
                data.setUint32(0x304, Number(begin), true);
                data.setUint32(0x30c, Number(begin + 4n), true);
            }
        }
        if (failure === "outside table") {
            if (width === 64) data.setInt32(0x733, 0x2000 - 0x737, true);
            else data.setUint32(0x73a, Number(fixture.base + 0x2000n), true);
        }
        if (failure === "truncated store") {
            // The instruction begins in the window, but its operand is beyond
            // the module end. A prefix-only search would accept it.
            fixture.image.fill(0, 0x700, 0x800);
            if (width === 64) {
                fixture.image.set([0x48, 0x8d, 0x0d], 0x1fe0);
                data.setInt32(0x1fe3, 0x600 - 0x1fe7, true);
                fixture.image.set([0x48, 0xc1, 0xe9, 3], 0x1ff0);
                fixture.image.set([0x48, 0x89, 0x05], 0x1ffd);
            } else {
                fixture.image.set([0x68, 0, 0, 0, 0, 0xe8], 0x1fe0);
                data.setUint32(0x1fe1, Number(fixture.base + 0x600n), true);
                fixture.image.set([0xc1, 0xea, 2, 0x52, 0xe8, 0, 0, 0, 0, 0xa3], 0x1ff6);
            }
        }
        if (failure === "absent name") fixture.image.fill(0, 0x600, 0x614);
        const host = await SplitScriptHost.instantiate(wasmPath);
        host.addProcess("game.exe", fixture.process);
        host.start();
        host.update(150);
        assert(!host.messages.includes("42"), `${failure} ${width}: invalid binding published state`);
        if (failure !== "absent name") assert(host.messages.some(message => message.startsWith("IL2CPP ")), `${failure}: missing rejection ${host.messages}`);
        host.setProcessOpen("game.exe", false);
        host.update();
        host.addProcess("game.exe", createIl2cppPeFixture(profile).process);
        host.updateUntil(() => host.messages.includes("42"), `reattach after ${failure} ${width}`, 2000);
        cases++;
    }
}
}
console.log(JSON.stringify({ cases }));
