import assert from 'node:assert/strict';
import { SplitScriptHost } from './support/splitscript_host.mjs';
const host = await SplitScriptHost.instantiate(process.argv[2]);
host.addProcess('game.exe');
host.start();
host.updateUntil(() => host.variables.has('done'), 'async failures complete');
for (const [key, value] of Object.entries({
    case0: 'immediate', case1: 'resumed', case2: 'propagated',
    case3: 'expression', case4: '42', outer: 'propagated',
    fallback: 'false', effects: '1', done: 'true',
})) assert.equal(host.variables.get(key), value, key);
host.update(3);
assert.equal(host.variables.get('effects'), '1', 'completed futures do not repeat side effects');
console.log('Async throw, propagation, completion, and discarded payload effects pass');
