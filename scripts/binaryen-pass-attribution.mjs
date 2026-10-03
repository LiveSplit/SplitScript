// Offline diagnosis only; Binaryen is not a compiler dependency.
// Generate target/size-check/corpus.json with the ignored write_size_corpus test first.
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const [optimizer, corpusPath = "target/size-check/corpus.json", output = "target/binaryen-attribution"] = process.argv.slice(2);
if (!optimizer) throw new Error("usage: node scripts/binaryen-pass-attribution.mjs <wasm-opt> [corpus.json] [output-directory]");
fs.mkdirSync(output, { recursive: true });
const features = ["--enable-gc", "--enable-reference-types", "--enable-bulk-memory", "--enable-multivalue", "--enable-mutable-globals", "--enable-sign-ext", "--enable-nontrapping-float-to-int", "--enable-extended-const"];
const groups = {
    inline: ["inlining-optimizing"],
    dae: ["dae-optimizing"],
    locals: ["ssa-nomerge", "simplify-locals-nostructure", "reorder-locals", "merge-locals", "coalesce-locals", "local-cse", "simplify-locals", "rse"],
    gc: ["global-refining", "gsi", "heap-store-optimization", "heap2local", "optimize-casts", "local-subtyping"],
    globals: ["simplify-globals-optimizing", "reorder-globals"],
    control: ["remove-unused-brs", "code-pushing", "code-folding", "merge-blocks"],
    merging: ["duplicate-function-elimination", "merge-similar-functions"],
};
function invoke(args, debug = false) {
    const result = spawnSync(optimizer, args, {
        encoding: "utf8", maxBuffer: 32 * 1024 * 1024,
        env: { ...process.env, BINARYEN_PASS_DEBUG: debug ? "1" : "0" },
    });
    if (result.error || result.status !== 0) throw new Error(`${result.error ?? result.stderr}`);
    return result;
}
function measure(source, args, destination) {
    invoke([source, ...features, ...args, "-o", destination]);
    const bytes = fs.readFileSync(destination);
    if (!WebAssembly.validate(bytes)) throw new Error(`Node validation failed: ${destination}`);
    return bytes.length;
}
const selected = new Set(["minish_cap", "unity_explicit", "celeste", "large_native"]);
const fixtures = JSON.parse(fs.readFileSync(corpusPath)).filter(f => selected.has(f.name));
if (!fixtures.length) throw new Error("No real-script fixtures in corpus");
const sourcePath = f => path.join(path.dirname(corpusPath), `${f.name}.release.wasm`);
const trace = invoke([sourcePath(fixtures[0]), ...features, "-Oz", "-o", path.join(output, "trace.wasm")], true);
fs.writeFileSync(path.join(output, "passes.log"), trace.stderr);
// Level 1 logs the outer passes, excluding nested function cleanup runners.
const passes = [...trace.stderr.matchAll(/running pass: ([\w-]+)\.\.\./g)].map(m => m[1]);
if (!passes.length) throw new Error("Binaryen did not report its pass sequence");
const report = { optimizer: invoke(["--version"]).stdout.trim(), features, passes, groups, fixtures: [] };
for (const fixture of fixtures) {
    const dir = path.join(output, fixture.name);
    fs.mkdirSync(dir, { recursive: true });
    const source = sourcePath(fixture);
    const row = {
        name: fixture.name, source: fixture.source, original: fs.statSync(source).size,
        roundtrip: measure(source, [], path.join(dir, "roundtrip.wasm")),
        // Optimization levels affect Binaryen's writer even without passes.
        // Compare standalone/prefix savings against matching writer settings.
        encodingBaseline: measure(source, ["--optimize-level=2", "--shrink-level=2"], path.join(dir, "encoding-baseline.wasm")),
        prefix: [], standalone: {}, skip: {},
    };
    // Each prefix starts from the original module and runs in one process:
    // serializing after every pass could change the IR seen by later passes.
    for (let i = 0; i < passes.length; i++) {
        const bytes = measure(source, ["--optimize-level=2", "--shrink-level=2", ...passes.slice(0, i + 1).map(p => `--${p}`)],
            path.join(dir, `prefix-${String(i).padStart(2, "0")}-${passes[i]}.wasm`));
        row.prefix.push({ pass: passes[i], bytes });
    }
    for (const pass of new Set(passes)) {
        row.standalone[pass] = measure(source, ["--optimize-level=2", "--shrink-level=2", `--${pass}`], path.join(dir, `single-${pass}.wasm`));
    }
    row.oz = measure(source, ["-Oz"], path.join(dir, "oz.wasm"));
    if (row.prefix.at(-1).bytes !== row.oz) throw new Error("Replayed pipeline differs from -Oz; review pass/options changes");
    for (const [name, skipped] of Object.entries(groups)) {
        row.skip[name] = measure(source, ["-Oz", ...skipped.flatMap(p => ["--skip-pass", p])], path.join(dir, `skip-${name}.wasm`));
    }
    report.fixtures.push(row);
    fs.writeFileSync(path.join(output, "report.json"), JSON.stringify(report, null, 2) + "\n");
    console.log(JSON.stringify({ name: row.name, original: row.original, roundtrip: row.roundtrip, encodingBaseline: row.encodingBaseline, oz: row.oz, skip: row.skip }));
}
