import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile, mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { resolveReleaseMetadata, preReleaseProperty } from '../scripts/release-metadata.mjs';
import { publicationNeeded, validatePublication } from '../scripts/marketplace-policy.mjs';
import { readPackagedRelease } from '../scripts/vsce-release.mjs';

const manifest = { publisher: 'LiveSplit', name: 'splitscript', version: '0.1.0' };
const commit = 'a'.repeat(40);
const checksum = 'b'.repeat(64);
const vsce = fileURLToPath(new URL('../vsce', import.meta.resolve('@vscode/vsce')));
const run = {
    event: 'push', conclusion: 'success', path: '.github/workflows/check.yml',
    head_repository: { full_name: 'LiveSplit/SplitScript' },
    head_sha: commit, head_branch: 'master',
};
const receipt = {
    id: 'LiveSplit.splitscript', version: '0.1.123', preRelease: true,
    marketplace: true, sourceCommit: commit, sourceRef: 'refs/heads/master', sha256: checksum,
};
const packaged = { id: receipt.id, version: receipt.version, preRelease: true };
const published = (version, preRelease) => ({
    version,
    properties: preRelease ? [{ key: preReleaseProperty, value: 'true' }] : [],
});

test('master previews generate increasing versions independently of the source manifest', () => {
    const context = { eventName: 'push', ref: 'refs/heads/master', runNumber: 123 };
    assert.deepEqual(resolveReleaseMetadata(manifest, context), {
        version: '0.1.123', preRelease: true, marketplace: true, vsixName: 'splitscript-latest.vsix',
    });
    const tags = ['latest', 'v0.2.0', 'v0.1.90', 'v0.10.0', 'v0.3.0', 'v0.12.0-beta'];
    assert.equal(resolveReleaseMetadata(manifest, { ...context, tags }).version, '0.11.123');
    assert.equal(resolveReleaseMetadata(manifest, { ...context, tags, runNumber: 124 }).version, '0.11.124');
    assert.equal(resolveReleaseMetadata(manifest, { ...context, tags, runAttempt: 2 }).version, '0.11.123');
});

test('version tags are full releases without a matching source version or parity restriction', () => {
    for (const version of ['0.2.0', '1.3.7']) {
        assert.deepEqual(resolveReleaseMetadata({ ...manifest, version: '0.1.0-development' }, {
            eventName: 'push', ref: `refs/tags/v${version}`,
        }), {
            version, preRelease: false, marketplace: true, vsixName: `splitscript-${version}.vsix`,
        });
    }
    assert.equal(resolveReleaseMetadata(manifest, { eventName: 'pull_request', ref: 'refs/pull/1/merge' }).marketplace, false);
    assert.equal(resolveReleaseMetadata(manifest, { eventName: 'push', ref: 'refs/heads/main' }).marketplace, false);
    assert.equal(resolveReleaseMetadata(manifest).version, manifest.version);
});

test('assembly uses the frozen workflow version and channel instead of recalculating them', () => {
    const environment = {
        ...process.env,
        SPLITSCRIPT_RELEASE_VERSION: '2.7.42',
        SPLITSCRIPT_PRE_RELEASE: 'false',
        SPLITSCRIPT_MARKETPLACE_RELEASE: 'true',
    };
    delete environment.GITHUB_OUTPUT;
    const result = spawnSync(process.execPath, [
        fileURLToPath(new URL('../scripts/release-metadata.mjs', import.meta.url)),
    ], { env: environment, encoding: 'utf8', shell: false });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout.trim(), 'SplitScript 2.7.42: stable');
});

test('invalid Marketplace tag versions and missing preview counters fail before a build', () => {
    for (const tag of ['v01.2.3', 'v1.2', 'v1.2.3-preview', 'v9007199254740992.0.0']) {
        assert.throws(() => resolveReleaseMetadata(manifest, { eventName: 'push', ref: `refs/tags/${tag}` }));
    }
    assert.throws(() => resolveReleaseMetadata(manifest, { eventName: 'push', ref: 'refs/heads/master' }));
});

test('publication requires the verified producer, matching bytes, and correct package metadata', () => {
    assert.equal(validatePublication(receipt, run, packaged, checksum), true);
    for (const changedRun of [
        { ...run, event: 'pull_request' }, { ...run, conclusion: 'failure' },
        { ...run, head_sha: 'c'.repeat(40) }, { ...run, head_branch: 'main' },
        { ...run, path: '.github/workflows/documentation.yml' },
        { ...run, head_repository: { full_name: 'someone/SplitScript' } },
    ]) {
        assert.throws(() => validatePublication(receipt, changedRun, packaged, checksum));
    }
    assert.throws(() => validatePublication(receipt, run, packaged, 'c'.repeat(64)));
    assert.throws(() => validatePublication(receipt, run, { ...packaged, preRelease: false }, checksum));
    assert.throws(() => validatePublication({ ...receipt, preRelease: false }, run, packaged, checksum));
    const full = { ...receipt, version: '1.3.7', preRelease: false, sourceRef: 'refs/tags/v1.3.7' };
    const fullPackage = { ...packaged, version: full.version, preRelease: false };
    assert.equal(validatePublication(full, { ...run, head_branch: 'v1.3.7' }, fullPackage, checksum), true);
    assert.equal(validatePublication(full, { ...run, head_branch: null }, fullPackage, checksum), true);
    assert.throws(() => validatePublication({ ...full, version: '1.3.8' }, { ...run, head_branch: null }, fullPackage, checksum));
});

test('reruns skip duplicates while full releases remain publishable below next-minor previews', () => {
    assert.equal(publicationNeeded(receipt, [published('0.1.123', true)]), false);
    assert.equal(publicationNeeded(receipt, [published('0.1.124', true)]), false);
    assert.equal(publicationNeeded(receipt, [published('0.1.122', true)]), true);
    assert.throws(() => publicationNeeded(receipt, [published('0.1.123', false)]), /other channel/u);
    assert.equal(publicationNeeded({ version: '0.2.0', preRelease: false }, [published('0.3.124', true)]), true);
});

test('real VSCE packages preserve both the stamped version and the selected channel', async () => {
    const temporary = await mkdtemp(join(tmpdir(), 'splitscript-release-test-'));
    try {
        const source = join(temporary, 'extension');
        await mkdir(source);
        const packageJson = {
            ...manifest, displayName: 'SplitScript', description: 'Release fixture',
            engines: { vscode: '^1.125.0' }, main: 'extension.js',
            activationEvents: [],
            files: ['extension.js'], license: 'MIT',
            repository: { type: 'git', url: 'https://github.com/LiveSplit/SplitScript.git' },
        };
        await writeFile(join(source, 'README.md'), '# Release fixture\n');
        await writeFile(join(source, 'extension.js'), 'exports.activate = () => {};\n');
        for (const [version, preRelease] of [['0.1.123', true], ['1.3.7', false]]) {
            await writeFile(join(source, 'package.json'), JSON.stringify({ ...packageJson, version }));
            const path = join(temporary, `${version}.vsix`);
            const result = spawnSync(process.execPath, [
                vsce, 'package', '--no-dependencies', '--skip-license',
                ...(preRelease ? ['--pre-release'] : []), '--out', path,
            ], { cwd: source, encoding: 'utf8', shell: false });
            assert.equal(result.status, 0, result.stderr || result.stdout);
            assert.deepEqual(await readPackagedRelease(path), { id: 'LiveSplit.splitscript', version, preRelease });
            assert((await readFile(path)).length > 0);
        }
    } finally {
        await rm(temporary, { recursive: true, force: true });
    }
});
