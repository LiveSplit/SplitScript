import assert from 'node:assert/strict';
import { publishVSIX } from '@vscode/vsce';
import { readVSIXPackage } from '@vscode/vsce/out/zip.js';
import { PublicGalleryAPI } from '@vscode/vsce/out/publicgalleryapi.js';
import { preReleaseProperty, validateVersion } from './release-metadata.mjs';

// VSCE has no public package-inspection API. Keep its validated ZIP/XML reader
// and gallery adapter here, tied to the pinned @vscode/vsce dependency; tests
// exercise real VSCE packages so a dependency update cannot silently change
// how the pre-release marker is interpreted.
export async function readPackagedRelease(path) {
    const { manifest, xmlManifest } = await readVSIXPackage(path);
    const metadata = xmlManifest.PackageManifest.Metadata[0];
    const identity = metadata.Identity[0].$;
    assert.equal(identity.Id, manifest.name, 'VSIX extension names disagree');
    assert.equal(identity.Publisher, manifest.publisher, 'VSIX publishers disagree');
    assert.equal(identity.Version, manifest.version, 'VSIX versions disagree');
    assert.equal(identity.TargetPlatform, undefined, 'the release VSIX must support all platforms');
    const marker = metadata.Properties?.[0].Property.find(property => property.$.Id === preReleaseProperty);
    if (marker !== undefined) assert.equal(marker.$.Value, 'true', 'invalid VSIX pre-release marker');
    return {
        id: `${manifest.publisher}.${manifest.name}`,
        version: validateVersion(manifest.version),
        preRelease: marker !== undefined,
    };
}

export async function publishedVersions() {
    // IncludeVersions (1) and IncludeVersionProperties (16), deliberately
    // including versions whose Marketplace validation is still pending.
    const gallery = new PublicGalleryAPI('https://marketplace.visualstudio.com');
    const extension = await gallery.getExtension('LiveSplit.splitscript', [1, 16]);
    return extension?.versions ?? [];
}

export async function publishPackagedRelease(path, release) {
    return publishVSIX(path, {
        azureCredential: true,
        preRelease: release.preRelease,
        skipDuplicate: true,
    });
}
