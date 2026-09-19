import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {SplitScriptHost} from './support/splitscript_host.mjs';
import {createMonoElfFixture} from './support/mono_elf_fixture.mjs';
const catalog = JSON.parse(await readFile(new URL('./fixtures/mono-elf-profiles.json', import.meta.url)));
const wasm = process.argv[2];
let cases = 0;
async function start(options) {
    const fixture = createMonoElfFixture(options);
    const host = await SplitScriptHost.instantiate(wasm, {operatingSystem: 'linux'});
    host.addProcess('game.exe', fixture.process); host.start();
    return {host, fixture};
}
async function run(label, options, expected = 'exact ELF build profile') {
    const {host, fixture} = await start(options);
    host.updateUntil(() => host.messages.includes('42'), label, 250);
    assert(host.messages.some(message => message.includes(expected)), `${label}: ${host.messages}`);
    assert(fixture.reads < 1000, `${label}: excessive reads ${fixture.reads}`);
    cases++;
    return fixture;
}
for (const profile of catalog.builds) {
    await run(profile.label, {version: profile.version,
        ...(profile.label.includes('UnityPlayer') ? {playerId: profile.build_id} : {runtimeId: profile.build_id})});
}
const oldId = catalog.builds.find(row => row.version === 'V1').build_id;
const v2Id = catalog.builds.find(row => row.version === 'V2').build_id;
const v3Id = catalog.builds.find(row => row.version === 'V3').build_id;
for (const version of ['V1', 'V1Cattrs', 'V2', 'V3']) {
    await run(`fallback ${version}`, {version, playerVersion: version === 'V3' ? '2021.2.0f1' : '2020.1.0f1'}, 'no readable ELF build ID');
}
for (const [playerVersion, version] of [['2021.1.0f1', 'V2'], ['2021.2.0f1', 'V3'], ['6000.7.0a3', 'V3'], ['2019.4.0f1', 'V2']]) {
    await run(`player version ${playerVersion}`, {version, playerVersion}, 'no readable ELF build ID');
}
await run('unknown runtime overrides known player', {version: 'V2', runtimeId: 'aabbccdd', playerId: v3Id, playerVersion: '2020.1.0f1'}, 'unknown ELF build');
const known = await run('known runtime overrides player', {version: 'V2', runtimeId: v2Id, playerId: v3Id});
assert.equal(known.playerReads, 0, 'runtime identity must suppress player identity reads');
await run('unreadable runtime identity permits player', {version: 'V3', malformedId: true, playerId: v3Id});
await run('unknown player fallback', {version: 'V3', playerId: 'aabbccdd', playerVersion: '2022.3.0f1'}, 'unknown ELF build');
await run('signed relative instruction', {version: 'V1', runtimeId: oldId, backward: true});
await run('false candidate skipped', {version: 'V2', runtimeId: v2Id, falseCandidate: true});
for (const options of [{machine: 183}, {elfClass: 1}, {signature: false}]) {
    const {host, fixture} = await start({version: 'V2', runtimeId: v2Id, ...options});
    host.updateUntil(() => host.messages.some(message => message.includes(options.signature === false ? 'instruction was not found' : 'requires a little-endian')), 'reject unsupported attachment', 50);
    const reads = fixture.reads; host.update(10);
    assert.equal(fixture.reads, reads); assert(!host.messages.includes('42'));
    host.setProcessOpen('game.exe', false); host.update();
    host.addProcess('game.exe', createMonoElfFixture({version: 'V2', runtimeId: v2Id}).process);
    host.updateUntil(() => host.messages.includes('42'), 'reattach after rejected runtime', 250);
    cases++;
}
{
    const {host, fixture} = await start({version: 'V2', runtimeId: v2Id, exportPresent: false});
    host.update(10); assert(!host.messages.includes('42'));
    host.setProcessOpen('game.exe', false); host.update(2);
    const reads = fixture.reads; host.update(5); assert.equal(fixture.reads, reads);
    host.addProcess('game.exe', createMonoElfFixture({version: 'V3', playerId: v3Id}).process);
    host.updateUntil(() => host.messages.includes('42'), 'cancel pending export and select new profile', 250);
    cases++;
}
console.log(JSON.stringify({linuxMonoCases: cases}));
