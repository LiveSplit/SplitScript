// Read-only validation of an already running Lunistice demo; does not launch the game.
// Usage: node scripts/probe-lunistice.mjs <lunistice.wasm>
import { createRequire } from "node:module";
import { SplitScriptHost } from "../tests/support/splitscript_host.mjs";
const require = createRequire(import.meta.url);
const native = require("../editors/vscode/dist/native/win32-x64/splitscript_process_native.node");
if (!process.argv[2]) throw new Error("usage: node scripts/probe-lunistice.mjs <lunistice.wasm>");
const name = "Lunistice-Demo.exe";
const handle = native.attachByName(name);
let reads = 0, failures = 0;
try {
    const host = await SplitScriptHost.instantiate(process.argv[2]);
    const modules = Object.fromEntries([name, "GameAssembly.dll", "UnityPlayer.dll"].map(name => [name, {
        address: BigInt(native.moduleAddress(handle, name)),
        size: BigInt(native.moduleSize(handle, name)),
        path: native.modulePath(handle, name),
    }]));
    const attached = host.addProcess(name, { modules, read({ address, length, outputPointer, host }) {
        reads++;
        try {
            const bytes = native.readProcessMemory(handle, address.toString(), length);
            if (bytes.length !== length) { failures++; return false; }
            host.bytes(outputPointer, length).set(bytes);
            return true;
        } catch { failures++; return false; }
    }});
    attached.executable.chunks = [];
    host.start();
    let ticks = 0;
    const started = performance.now();
    for (; ticks < 20000 && host.variables.size < 5 && native.isOpen(handle); ticks++) {
        host.update();
        if (ticks % 100 === 0) await new Promise(resolve => setTimeout(resolve, 1));
    }
    console.log(host.json({ ticks, milliseconds: performance.now() - started, reads, failures, ...host.summary() }));
    if (host.variables.size < 5) process.exitCode = 1;
} finally {
    native.detach(handle);
}
