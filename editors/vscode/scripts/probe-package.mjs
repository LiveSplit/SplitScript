import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { nativeBridgeFileName, supportedNativePlatforms } from './native-platforms.mjs';
import { stageExtension } from './stage-extension.mjs';
import { readReleaseMetadata } from './release-metadata.mjs';
import { readPackagedRelease } from './vsce-release.mjs';

const extension = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(extension, '..', '..');
const commandArguments = process.argv.slice(2);
const keepPackage = commandArguments.includes('--keep-package');
const positionalArguments = commandArguments.filter(argument => argument !== '--keep-package');
assert(
    positionalArguments.every(argument => !argument.startsWith('--')),
    `unknown package probe option ${positionalArguments.find(argument => argument.startsWith('--'))}`,
);
assert(positionalArguments.length <= 2, 'expected at most a production dist and package output');
const [suppliedProductionDist, suppliedPackageOutput] = positionalArguments;
const release = await readReleaseMetadata();
const productionDist = resolve(
    suppliedProductionDist ?? resolve(repository, 'target', 'vscode-package', 'dist'),
);
const vsce = resolve(extension, 'node_modules', '@vscode', 'vsce', 'vsce');
const supportedPlatforms = new Set(supportedNativePlatforms);
const maxCompilerWasmBytes = 8 * 1024 * 1024;
const maxVsixBytes = 12 * 1024 * 1024;
const configuredPlatforms = process.env.SPLITSCRIPT_REQUIRED_NATIVE_PLATFORMS;
const requiredPlatforms = configuredPlatforms === undefined
    ? process.env.SPLITSCRIPT_NATIVE_ARTIFACTS === undefined
        ? [`${process.platform}-${process.arch}`]
        : supportedNativePlatforms
    : configuredPlatforms.split(',').map(value => value.trim()).filter(Boolean);

if (suppliedProductionDist === undefined) {
    const build = spawnSync(process.execPath, [
        resolve(extension, 'scripts', 'build.mjs'),
        '--production',
        '--output',
        productionDist,
    ], {
        cwd: extension,
        stdio: 'inherit',
        shell: false,
    });
    if (build.error) throw build.error;
    assert.equal(build.status, 0, 'the isolated production extension build failed');
}

const temporary = await mkdtemp(join(tmpdir(), 'splitscript-vsix-probe-'));
try {
    const staging = resolve(temporary, 'extension');
    // VSCE unconditionally invokes vscode:prepublish while packaging. The
    // staged build script recognizes this marker and leaves the exact audited
    // production tree untouched.
    await stageExtension(extension, productionDist, staging, true);
    // Stamp only the disposable packaging manifest. Branch/tag metadata
    // determines the distribution version and channel; source edits are not
    // required to publish either kind of release.
    const manifestPath = resolve(staging, 'package.json');
    const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
    manifest.version = release.version;
    await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

    const listing = spawnSync(process.execPath, [vsce, 'ls', '--no-dependencies'], {
        cwd: staging,
        encoding: 'utf8',
        shell: false,
    });
    if (listing.error) throw listing.error;
    assert.equal(listing.status, 0, listing.stderr);
    const files = listing.stdout
        .split(/\r?\n/u)
        .map(file => file.trim().replaceAll('\\', '/'))
        .filter(Boolean);
    const fileSet = new Set(files);

    for (const file of [
        'package.json',
        'README.md',
        'CHANGELOG.md',
        'SUPPORT.md',
        'LICENSE',
        'language-configuration.json',
        'syntaxes/splitscript.tmLanguage.json',
        'styles/documentation.css',
        'media/icon.png',
        'media/splitscript-debugger.svg',
        'dist/splitscript_vscode_wasm.wasm',
        'dist/extension.js',
        'dist/embeddedCompilerNodeWorker.js',
        'dist/embeddedLanguageServerNodeWorker.js',
        'dist/runtimeProbeWorker.js',
        'dist/runtimeWorker.js',
        'dist/web/extension.js',
        'dist/web/embeddedCompilerWorker.js',
        'dist/web/embeddedLanguageServerWorker.js',
    ]) {
        assert(fileSet.has(file), `required extension artifact ${file} is missing`);
    }

    for (const platform of requiredPlatforms) {
        assert(supportedPlatforms.has(platform), `unknown required native platform ${platform}`);
        assert(
            fileSet.has(`dist/native/${platform}/${nativeBridgeFileName}`),
            `the ${platform} native process bridge is missing from the extension package file list`,
        );
    }

    for (const file of files) {
        assert(
            !file.startsWith('/') && !file.split('/').includes('..'),
            `unsafe package path ${file}`,
        );
        assert(!file.endsWith('.map'), `production source map was packaged: ${file}`);
        assert(!file.startsWith('dist/web/test/'), `web-host test artifact was packaged: ${file}`);
        assert(
            !/^(?:src|test|test-workspace|scripts|node_modules)\//u.test(file),
            `development-only file was packaged: ${file}`,
        );
        assert(
            !['package-lock.json', 'tsconfig.json', 'tsconfig.node.json', 'tsconfig.test.json']
                .includes(file),
            `development-only manifest was packaged: ${file}`,
        );

        if (file.endsWith('.node')) {
            const parts = file.split('/');
            assert.equal(parts.length, 4, `unexpected native binary in extension package: ${file}`);
            const unexpectedNative = `unexpected native binary in extension package: ${file}`;
            assert.equal(parts[0], 'dist', unexpectedNative);
            assert.equal(parts[1], 'native', unexpectedNative);
            assert.equal(
                parts[3],
                nativeBridgeFileName,
                `unexpected native binary in extension package: ${file}`,
            );
            assert(
                supportedPlatforms.has(parts[2]),
                `unsupported native bridge platform ${parts[2]}`,
            );
        } else {
            assert(
                !/\.(?:exe|dll|so|dylib)$/iu.test(file),
                `unexpected native binary in extension package: ${file}`,
            );
        }
    }

    const compilerWasm = await readFile(resolve(staging, 'dist', 'splitscript_vscode_wasm.wasm'));
    assert.deepEqual(
        [...compilerWasm.subarray(0, 4)],
        [0x00, 0x61, 0x73, 0x6d],
        'the packaged compiler is not a WebAssembly module',
    );
    const expectedCompilerWasm = await readFile(resolve(
        repository,
        'target',
        'wasm32-unknown-unknown',
        'max-opt',
        'splitscript_vscode_wasm.wasm',
    ));
    assert.deepEqual(
        compilerWasm,
        expectedCompilerWasm,
        'the packaged compiler Wasm is stale relative to the max-opt build',
    );
    assert(
        compilerWasm.byteLength <= maxCompilerWasmBytes,
        `the optimized compiler Wasm exceeds its ${maxCompilerWasmBytes}-byte package budget`,
    );

    for (const [entrypoint, localArtifacts] of [
        ['dist/extension.js', [
            'splitscript_vscode_wasm.wasm',
            'embeddedCompilerNodeWorker.js',
            'embeddedLanguageServerNodeWorker.js',
        ]],
        ['dist/web/extension.js', [
            'splitscript_vscode_wasm.wasm',
            'embeddedCompilerWorker.js',
            'embeddedLanguageServerWorker.js',
        ]],
    ]) {
        const source = await readFile(resolve(staging, entrypoint), 'utf8');
        for (const artifact of localArtifacts) {
            assert(
                source.includes(artifact),
                `${entrypoint} does not load packaged local artifact ${artifact}`,
            );
        }
    }

    const output = resolve(suppliedPackageOutput ?? (keepPackage
        ? resolve(extension, `splitscript-${release.version}.vsix`)
        : resolve(temporary, 'splitscript-probe.vsix')));
    const packaged = spawnSync(process.execPath, [
        vsce,
        'package',
        '--no-dependencies',
        ...(release.preRelease ? ['--pre-release'] : []),
        '--out',
        output,
    ], {
        cwd: staging,
        encoding: 'utf8',
        shell: false,
        env: {
            ...process.env,
            SPLITSCRIPT_PACKAGE_PREBUILT: '1',
        },
    });
    if (packaged.error) throw packaged.error;
    assert.equal(packaged.status, 0, packaged.stderr || packaged.stdout);
    const packageSize = (await stat(output)).size;
    assert(packageSize > 0);
    assert(
        packageSize <= maxVsixBytes,
        `the VSIX exceeds its ${maxVsixBytes}-byte distribution budget`,
    );
    const packageBytes = await readFile(output);
    assert.deepEqual(await readPackagedRelease(output), {
        id: 'LiveSplit.splitscript',
        version: release.version,
        preRelease: release.preRelease,
    }, 'the packaged release version or channel is incorrect');
    await writeFile(`${output}.json`, `${JSON.stringify({
        id: 'LiveSplit.splitscript',
        version: release.version,
        preRelease: release.preRelease,
        marketplace: release.marketplace,
        sourceCommit: process.env.SPLITSCRIPT_GIT_REVISION ?? null,
        sourceRef: process.env.GITHUB_REF ?? null,
        sha256: createHash('sha256').update(packageBytes).digest('hex'),
    }, null, 2)}\n`);
    console.log(
        `VSIX packaging probe passed with ${files.length} production files and `
        + `${requiredPlatforms.length} required native bridge artifact(s); `
        + `${compilerWasm.byteLength} compiler bytes and ${packageSize} packaged bytes`
        + ` as ${release.preRelease ? 'a pre-release' : 'a stable release'} ${release.version}.`,
    );
} finally {
    await rm(temporary, { recursive: true, force: true });
}
