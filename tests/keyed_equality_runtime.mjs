import assert from 'node:assert/strict';
import { SplitScriptHost } from './support/splitscript_host.mjs';
const host = await SplitScriptHost.instantiate(process.argv[2]);
host.addProcess('game.exe', {read: () => null}); host.start();
host.updateUntil(() => host.variables.has('empty map'), 'collection equality');
const expected = ['set order', 'set count', 'set spare capacity', 'set contents', 'map order',
    'nested duplicate', 'map key', 'optional map', 'array map', 'map values', 'map keys',
    'one to one', 'pair matching', 'NaN set', 'NaN map', 'empty set', 'empty map'];
assert.equal(host.variables.size, expected.length);
for (const name of expected) assert.equal(host.variables.get(name), 'true', name);
console.log(JSON.stringify({keyedEqualityCases: expected.length}));
