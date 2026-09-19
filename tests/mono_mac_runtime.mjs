import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {SplitScriptHost} from './support/splitscript_host.mjs';
import {createMonoMachFixture} from './support/mono_mach_fixture.mjs';
const catalog = JSON.parse(await readFile(new URL('./fixtures/mono-mach-profiles.json', import.meta.url)));
const wasm = process.argv[2];
let cases = 0;
async function start(options) {
    const fixture = createMonoMachFixture(options);
    const host = await SplitScriptHost.instantiate(wasm, {operatingSystem: 'macos'});
    host.addProcess('game.exe', fixture.process); host.start();
    return {host, fixture};
}
async function run(label, options, expected = 'exact Mach-O UUID profile') {
    const {host, fixture} = await start(options);
    host.updateUntil(() => host.messages.includes('42'), label, 250);
    assert(host.messages.some(message => message.includes(expected)), `${label}: ${host.messages}`);
    assert(fixture.reads < 1000, `${label}: excessive reads ${fixture.reads}`);
    cases++;
    return fixture;
}
for (const arm of [false, true]) {
    const uuid = catalog.builds.find(row => row.architecture === (arm ? 'arm64' : 'x86_64')).uuid;
    for (const mode of ['valid', 'negative page', 'positive page', 'wide load', 'false candidate']) {
        await run(`${arm}/${mode}`, {arm, uuid, mode});
    }
    const exact = await run('exact UUID overrides player', {arm, uuid, playerVersion: '2020.1.0f1'});
    assert.equal(exact.playerReads, 0, 'exact UUID must skip player-version discovery');
    for (const version of ['V1', 'V1Cattrs', 'V2', 'V3']) {
        await run(`${arm}/fallback ${version}`, {arm, version, playerVersion: version === 'V3' ? '6000.5.10f1' : '2020.1.0f1'}, 'no Mach-O UUID');
    }
    await run('unknown UUID fallback', {arm, uuid: '00112233-4455-6677-8899-AABBCCDDEEFF', playerVersion: '2021.2.0f1'}, 'unknown Mach-O UUID');
    await run('unreadable UUID fallback', {arm, uuid, malformedUuid: true, playerVersion: '2021.2.0f1'}, 'unreadable Mach-O UUID');
    const rejectionModes = arm ? ['unaligned', 'truncated', 'missing', 'wrong ADRP register', 'wrong LDR register', 'wrong destination', 'wrong width', 'not ADRP', 'not unsigned LDR', 'maximum positive page', 'maximum negative page', 'outside load', 'unreadable instructions'] : ['missing'];
    for (const mode of rejectionModes) {
        const {host, fixture} = await start({arm, uuid, mode});
        host.updateUntil(() => host.messages.some(message => message.includes('instruction was not found') || message.includes('assembly export is invalid')), mode, 100);
        const before = fixture.reads; host.update(10);
        assert.equal(fixture.reads, before, mode); assert(!host.messages.includes('42'), mode);
        host.setProcessOpen('game.exe', false); host.update();
        host.addProcess('game.exe', createMonoMachFixture({arm, uuid}).process);
        host.updateUntil(() => host.messages.includes('42'), `recover ${mode}`, 250);
        cases++;
    }
    if (arm) for (const [base, mode] of [[0x1000n, 'maximum negative page'], [0xffffffffffffe000n, 'maximum positive page']]) {
        const {host, fixture} = await start({arm, uuid, base, mode});
        host.updateUntil(() => host.messages.some(message => message.includes('instruction was not found')), 'address wrap rejected at ' + base, 100);
        const before = fixture.reads; host.update(5); assert.equal(fixture.reads, before); assert(!host.messages.includes('42')); cases++;
    }
    for (const machine of [0x1000007, 0x100000c, 7]) {
        if (machine === (arm ? 0x100000c : 0x1000007)) continue;
        const {host, fixture} = await start({arm, uuid, machine});
        host.updateUntil(() => host.messages.some(message => message.includes('wrong CPU architecture') || message.includes('requires an x86-64')), 'CPU mismatch');
        const before = fixture.reads; host.update(10); assert.equal(fixture.reads, before); assert(!host.messages.includes('42')); cases++;
    }
    const {host, fixture} = await start({arm, uuid, mode: 'missing export'});
    host.update(10); assert(!host.messages.includes('42'));
    host.setProcessOpen('game.exe', false); host.update(2);
    const before = fixture.reads; host.update(5); assert.equal(fixture.reads, before);
    const nextUuid = catalog.builds.find(row => row.uuid !== uuid).uuid;
    host.addProcess('game.exe', createMonoMachFixture({arm: !arm, uuid: nextUuid}).process);
    host.updateUntil(() => host.messages.includes('42'), 'reattach different CPU slice', 250); cases++;
}
console.log(JSON.stringify({macMonoCases: cases}));
