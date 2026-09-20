import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {SplitScriptHost} from './support/splitscript_host.mjs';
import {createMonoElfFixture} from './support/mono_elf_fixture.mjs';
import {createMonoMachFixture} from './support/mono_mach_fixture.mjs';

const [wasm, platform, version] = process.argv.slice(2);
assert(['linux', 'mac'].includes(platform));
assert(['V1', 'V1Cattrs', 'V2', 'V3'].includes(version));
const linux = platform === 'linux';
const catalog = JSON.parse(await readFile(new URL(`./fixtures/mono-${linux ? 'elf' : 'mach'}-profiles.json`, import.meta.url)));
const allowedModules = linux ? ['libmono.so', 'libmonobdwgc-2.0.so'] : ['libmono.0.dylib', 'libmonobdwgc-2.0.dylib'];
const make = options => (linux ? createMonoElfFixture : createMonoMachFixture)({version, ...options});
let cases = 0;
async function start(options, {delayModule = false, delayHeader = false} = {}) {
    const fixture = make(options);
    const base = Object.values(fixture.process.modules)[0].address;
    let readable = !delayHeader;
    const read = fixture.process.read;
    fixture.process.read = request => {
        const at = BigInt.asUintN(64, request.address), end = at + BigInt(request.length);
        const identityStart = base + (linux ? 0x900n : 216n);
        const identityEnd = identityStart + (linux ? 32n : 16n);
        assert(end <= identityStart || at >= identityEnd, `${platform}/${version}: identity payload was read`);
        if (!readable && at === base) return false;
        return read(request);
    };
    const host = await SplitScriptHost.instantiate(wasm, {operatingSystem: linux ? 'linux' : 'macos'});
    const module = host.module.bind(host);
    host.module = (handle, name) => {
        assert(allowedModules.includes(name), `${platform}/${version}: queried unrelated module ${name}`);
        return module(handle, name);
    };
    const process = host.addProcess('game.exe', fixture.process);
    const modules = new Map(process.modules);
    if (delayModule) process.modules.clear();
    host.start();
    return {host, fixture, resume() {readable = true; process.modules = modules;}};
}
async function success(label, options = {}, delay = {}) {
    const result = await start(options, delay);
    if (delay.delayModule || delay.delayHeader) {
        result.host.update(8);
        assert(!result.host.messages.includes('42'), label);
        result.resume();
    }
    result.host.updateUntil(() => result.host.messages.includes('42'), `${platform}/${version}/${label}`, 250);
    assert.equal(result.fixture.playerReads, 0, label);
    assert(result.host.messages.every(message => message === '42'), `${label}: ${result.host.messages}`);
    cases++;
    return result;
}
for (const arm of linux ? [false] : [false, true]) {
    await success('without player or identity', {arm});
    const contraryIdentity = linux
        ? {runtimeId: catalog.builds.find(row => row.version !== version).build_id}
        : {uuid: catalog.builds.find(row => row.architecture === (arm ? 'x86_64' : 'arm64')).uuid};
    await success('explicit selection overrides contradictory identity and player', {arm, ...contraryIdentity, playerVersion: version === 'V3' ? '2020.1.0f1' : '6000.5.10f1'});
    await success('delayed module', {arm}, {delayModule: true});
    await success('delayed header', {arm}, {delayHeader: true});
    await success('signed assembly instruction', linux ? {backward: true} : {arm, mode: 'negative page'});
    const failures = linux ? [{machine: 183}, {elfClass: 1}, {signature: false}]
        : [{arm, machine: 7}, {arm, mode: 'missing'}];
    for (const options of failures) {
        const {host, fixture} = await start(options);
        host.updateUntil(() => host.messages.some(message => /requires|instruction was not found/.test(message)), 'reject target', 100);
        const reads = fixture.reads; host.update(8);
        assert.equal(fixture.reads, reads, 'rejected attachment must stop reading');
        assert(!host.messages.includes('42'));
        host.setProcessOpen('game.exe', false); host.update(2);
        host.addProcess('game.exe', make({arm}).process);
        host.updateUntil(() => host.messages.includes('42'), 'recover after rejected target', 250);
        cases++;
    }
    const {host, fixture} = await start(linux ? {exportPresent: false} : {arm, mode: 'missing export'});
    host.update(8); assert(!host.messages.includes('42'));
    host.setProcessOpen('game.exe', false); host.update(2);
    const reads = fixture.reads; host.update(5); assert.equal(fixture.reads, reads, 'cancel pending export');
    host.addProcess('game.exe', make({arm: !arm}).process);
    host.updateUntil(() => host.messages.includes('42'), 'reattach with a fresh runtime', 250);
    cases++;
}
console.log(JSON.stringify({explicitMonoPlatform: platform, version, cases}));
