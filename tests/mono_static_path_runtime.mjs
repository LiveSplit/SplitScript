import assert from 'node:assert/strict';
import {SplitScriptHost} from './support/splitscript_host.mjs';
import {createMonoV2Fixture} from './support/mono_v2_fixture.mjs';
let cases = 0;
for (const name of ['Instance', '<Instance>k__BackingField']) {
    const fixture = createMonoV2Fixture({
        className: 'LevelFlowService', fields: [{name: '_state', offset: 0x20}],
        parent: {className: 'BaseService', fields: [{name, offset: 0x10}]},
    });
    const slot = fixture.parentStaticTable + 0x10n;
    fixture.writeU64(slot, 0xc000n); fixture.writeI32(0xc020n, 42);
    fixture.writeU64(fixture.staticTable + 0x10n, 0xd000n); fixture.writeI32(0xd020n, 99);
    // Initialization of the declaring class's static table may lag attachment.
    fixture.writeU64(0x9c48n, 0n);
    const host = await SplitScriptHost.instantiate(process.argv[2]);
    let reads = 0;
    const read = fixture.process.read;
    fixture.process.read = request => { reads++; return read(request); };
    host.addProcess('game.exe', fixture.process); host.start(); host.update(8);
    assert(!host.variables.has('slot'), `${name}: path published before owner initialization`);
    fixture.writeU64(0x9c48n, fixture.parentStaticTable);
    host.updateUntil(() => host.variables.get('value') === '42', name);
    assert.equal(BigInt(host.variables.get('slot')), slot, 'path must start in declaring class storage');
    fixture.writeU64(slot, 0xe000n); fixture.writeI32(0xe020n, 84);
    host.updateUntil(() => host.variables.get('value') === '84', 'replacement singleton');
    fixture.writeU64(slot, 0n);
    host.updateUntil(() => host.variables.get('status') === 'error', 'null singleton');
    fixture.writeU64(slot, 0xc000n);
    host.updateUntil(() => host.variables.get('value') === '42' && host.variables.get('status') === 'ok', 'recovered singleton');
    host.setProcessOpen('game.exe', false); host.update(2);
    const stopped = reads; host.update(3); assert.equal(reads, stopped, 'closed process must not be read');
    host.setProcessOpen('game.exe', true); host.variables.clear();
    host.updateUntil(() => host.variables.get('value') === '42', 'reattach static path');
    cases++;
}
console.log(JSON.stringify({monoStaticPathCases: cases}));
