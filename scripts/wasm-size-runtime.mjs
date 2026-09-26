// Run maintained behavioral scenarios against baseline and optimized artifacts.
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const directory = path.resolve("target/size-check");
const corpus = JSON.parse(fs.readFileSync(path.join(directory, "corpus.json"), "utf8"));
const fixtures = [...fs.readFileSync("src/bin/xtask.rs", "utf8").matchAll(
    /RuntimeFixture\s*\{\s*source:\s*"([^"]+)",\s*output:\s*"([^"]+)",\s*profile:\s*"([^"]+)",\s*harness:\s*"([^"]+)",\s*extra_arguments:\s*&\[([^\]]*)\]/g,
)];
const variants = process.argv.slice(2);
if (!variants.length) variants.push("baseline", "release");
let scenarios = 0;
for (const row of corpus) {
    let selected = fixtures.filter(match => match[1] === row.source && match[3] === "release");
    if (!selected.length) selected = fixtures.filter(match => match[1] === row.source);
    if (!selected.length) throw new Error(`No maintained scenarios for ${row.source}`);
    for (const variant of variants) {
        for (const fixture of selected) {
            const args = [...fixture[5].matchAll(/"([^"]*)"/g)].map(match => match[1]);
            const result = spawnSync(process.execPath, [fixture[4], path.join(directory, `${row.name}.${variant}.wasm`), ...args], { encoding: "utf8" });
            if (result.status !== 0) throw new Error(`${row.name}/${variant}/${args.join(" ")}: ${result.stdout}\n${result.stderr}`);
            scenarios++;
        }
        console.log(`${row.name}/${variant}: ${selected.length} scenarios passed`);
    }
}
console.log(`${scenarios} maintained runtime scenarios passed`);
