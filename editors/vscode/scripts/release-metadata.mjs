import assert from 'node:assert/strict';
import { appendFile, readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

export const preReleaseProperty = 'Microsoft.VisualStudio.Code.PreRelease';

export function validateVersion(version) {
    assert(
        typeof version === 'string' && /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u.test(version)
            && version.split('.').every(part => Number.isSafeInteger(Number(part))),
        'Marketplace versions must be numeric major.minor.patch versions without suffixes',
    );
    return version;
}

export function compareVersions(left, right) {
    const leftParts = validateVersion(left).split('.').map(Number);
    const rightParts = validateVersion(right).split('.').map(Number);
    for (let index = 0; index < leftParts.length; index++) {
        if (leftParts[index] !== rightParts[index]) {
            return Math.sign(leftParts[index] - rightParts[index]);
        }
    }
    return 0;
}

export function versionFromTag(tag) {
    const match = /^v(.+)$/u.exec(tag);
    assert(match, 'full release tags must use v<major>.<minor>.<patch>');
    return validateVersion(match[1]);
}

export function channelForRef(ref) {
    assert.equal(typeof ref, 'string', 'release source ref is missing');
    if (ref === 'refs/heads/master') return true;
    assert(ref.startsWith('refs/tags/v'), 'Marketplace builds must come from master or a version tag');
    versionFromTag(ref.slice('refs/tags/'.length));
    return false;
}

// Full release versions come from tags; master gets an automatically versioned
// preview of the next minor series. Before the first full tag, previews use
// 0.1.<run number>. GitHub's workflow run counter is unchanged on a rerun;
// upload retries reuse the original artifact. Source versions do not constrain
// either channel, and no source edits or preview tags are required.
export function resolveReleaseMetadata(manifest, context = {}) {
    assert.equal(manifest.publisher, 'LiveSplit', 'unexpected extension publisher');
    assert.equal(manifest.name, 'splitscript', 'unexpected extension name');
    const { eventName, ref, runNumber, tags = [] } = context;
    let version = manifest.version;
    let preRelease = true;
    let marketplace = false;
    if (eventName === 'push' && ref === 'refs/heads/master') {
        assert(Number.isSafeInteger(runNumber) && runNumber > 0, 'preview builds need a positive CI run number');
        const versions = tags.filter(tag => /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u.test(tag))
            .map(versionFromTag).sort(compareVersions);
        const base = versions.at(-1) ?? '0.0.0';
        const [major, minor] = base.split('.').map(Number);
        version = validateVersion(`${major}.${minor + 1}.${runNumber}`);
        marketplace = true;
    } else if (eventName === 'push' && ref?.startsWith('refs/tags/')) {
        version = versionFromTag(ref.slice('refs/tags/'.length));
        preRelease = channelForRef(ref);
        marketplace = true;
    }
    return {
        version: validateVersion(version),
        preRelease,
        marketplace,
        vsixName: preRelease ? 'splitscript-latest.vsix' : `splitscript-${version}.vsix`,
    };
}

export async function readReleaseMetadata() {
    const manifest = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));
    if (process.env.SPLITSCRIPT_RELEASE_VERSION !== undefined) {
        // Freeze metadata once per workflow. Assembly must not recalculate its
        // preview series if another tag appears while the native jobs build.
        const preRelease = process.env.SPLITSCRIPT_PRE_RELEASE;
        const marketplace = process.env.SPLITSCRIPT_MARKETPLACE_RELEASE;
        assert(['true', 'false'].includes(preRelease), 'release channel is missing');
        assert(['true', 'false'].includes(marketplace), 'publication policy is missing');
        const version = validateVersion(process.env.SPLITSCRIPT_RELEASE_VERSION);
        return {
            version,
            preRelease: preRelease === 'true',
            marketplace: marketplace === 'true',
            vsixName: preRelease === 'true' ? 'splitscript-latest.vsix' : `splitscript-${version}.vsix`,
        };
    }
    const git = spawnSync('git', ['tag', '--list', 'v*'], {
        cwd: fileURLToPath(new URL('../../..', import.meta.url)),
        encoding: 'utf8',
        shell: false,
    });
    if (process.env.GITHUB_EVENT_NAME === 'push') {
        if (git.error) throw git.error;
        assert.equal(git.status, 0, git.stderr);
    }
    return resolveReleaseMetadata(manifest, {
        eventName: process.env.GITHUB_EVENT_NAME,
        ref: process.env.GITHUB_REF,
        runNumber: Number(process.env.GITHUB_RUN_NUMBER),
        tags: (git.stdout ?? '').trim().split(/\r?\n/u),
    });
}

export async function writeActionOutputs(values) {
    if (process.env.GITHUB_OUTPUT !== undefined) {
        await appendFile(process.env.GITHUB_OUTPUT, Object.entries(values)
            .map(([key, value]) => `${key}=${value}\n`).join(''));
    }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    const release = await readReleaseMetadata();
    console.log(`SplitScript ${release.version}: ${release.preRelease ? 'pre-release' : 'stable'}`);
    await writeActionOutputs({
        version: release.version,
        pre_release: release.preRelease,
        marketplace: release.marketplace,
        vsix_name: release.vsixName,
    });
}
