import assert from 'node:assert/strict';
import { channelForRef, compareVersions, preReleaseProperty, validateVersion, versionFromTag } from './release-metadata.mjs';

export function validatePublication(receipt, run, packaged, sha256) {
    assert.equal(run.event, 'push', 'only verified pushes can publish');
    assert.equal(run.conclusion, 'success', 'the Check workflow must succeed before publication');
    assert.equal(run.path?.split('@')[0], '.github/workflows/check.yml', 'unexpected verification workflow');
    assert.equal(run.head_repository?.full_name, 'LiveSplit/SplitScript', 'unexpected source repository');
    assert.match(run.head_sha, /^[a-f0-9]{40}$/u, 'verification commit is missing');
    assert.equal(receipt.sourceCommit, run.head_sha, 'the VSIX is from a different commit');
    assert.equal(receipt.id, 'LiveSplit.splitscript', 'unexpected receipt extension identity');
    assert.equal(receipt.sha256, sha256, 'the audited VSIX checksum does not match');
    validateVersion(receipt.version);
    assert.equal(typeof receipt.marketplace, 'boolean', 'publication policy is missing');
    if (!receipt.marketplace) return false;

    assert.equal(receipt.preRelease, channelForRef(receipt.sourceRef), 'the VSIX is in the wrong release channel');
    const refName = receipt.sourceRef.replace(/^refs\/(?:heads|tags)\//u, '');
    // GitHub can omit head_branch for a tag push. In that case the receipt
    // still binds the tested version-tag build to this exact successful run.
    if (run.head_branch !== null || receipt.preRelease) {
        assert.equal(run.head_branch, refName, 'the VSIX is from a different branch or tag');
    }
    if (!receipt.preRelease) {
        assert.equal(receipt.version, versionFromTag(refName), 'the packaged version does not match the tag');
    }
    assert.deepEqual(packaged, {
        id: receipt.id,
        version: receipt.version,
        preRelease: receipt.preRelease,
    }, 'the VSIX metadata does not match the audit receipt');
    return true;
}

export function publicationNeeded(release, versions) {
    const existing = versions.filter(version => version.version === release.version);
    for (const version of existing) {
        assert.equal(
            version.properties?.some(property => property.key === preReleaseProperty && property.value === 'true') ?? false,
            release.preRelease,
            `version ${release.version} is already published in the other channel; choose an unused version`,
        );
    }
    if (existing.length > 0) return false;
    if (release.preRelease && versions.some(version =>
        version.properties?.some(property => property.key === preReleaseProperty && property.value === 'true')
            && compareVersions(version.version, release.version) > 0)) {
        return false;
    }
    // A full release can be numerically below the next-minor preview series.
    // Stable users must still receive that full release.
    return true;
}
