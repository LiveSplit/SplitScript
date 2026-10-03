import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { appendFile, readFile, readdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { readPackagedRelease, publishedVersions, publishPackagedRelease } from './vsce-release.mjs';
import { publicationNeeded, validatePublication } from './marketplace-policy.mjs';
import { writeActionOutputs } from './release-metadata.mjs';

const arguments_ = process.argv.slice(2);
const checkOnly = arguments_[0] === '--check';
if (checkOnly) arguments_.shift();
assert.equal(arguments_.length, 1, 'expected [--check] <audited artifact directory>');
assert.equal(process.env.GITHUB_EVENT_NAME, 'workflow_run', 'publication must follow the Check workflow');
const { workflow_run: run } = JSON.parse(await readFile(process.env.GITHUB_EVENT_PATH, 'utf8'));
const directory = resolve(arguments_[0]);
const packages = (await readdir(directory)).filter(file => file.endsWith('.vsix'));
assert.equal(packages.length, 1, 'expected one audited VSIX');
const packagePath = resolve(directory, packages[0]);
const receipt = JSON.parse(await readFile(`${packagePath}.json`, 'utf8'));
const sha256 = createHash('sha256').update(await readFile(packagePath)).digest('hex');
const release = await readPackagedRelease(packagePath);
let publish = validatePublication(receipt, run, release, sha256);
let reason = publish ? 'Ready to publish' : 'This build is not a Marketplace release';

if (publish && release.preRelease) {
    const response = await fetch('https://api.github.com/repos/LiveSplit/SplitScript/git/ref/heads/master', {
        headers: {
            Accept: 'application/vnd.github+json',
            Authorization: `Bearer ${process.env.GH_TOKEN}`,
        },
        signal: AbortSignal.timeout(30_000),
    });
    assert(response.ok, `could not check the current master commit (${response.status})`);
    if ((await response.json()).object.sha !== receipt.sourceCommit) {
        publish = false;
        reason = 'A newer master commit superseded this preview';
    }
}
if (publish && !publicationNeeded(release, await publishedVersions())) {
    publish = false;
    reason = 'The version is already published or a newer preview is available';
}
if (!checkOnly && publish) {
    const result = await publishPackagedRelease(packagePath, release);
    reason = result === undefined ? 'The version was already published' : 'Uploaded; Marketplace validation may still be pending';
}
console.log(`${reason}: ${release.id} ${release.version} (${release.preRelease ? 'pre-release' : 'stable'}).`);
await writeActionOutputs({ publish, version: release.version, pre_release: release.preRelease });
if (process.env.GITHUB_STEP_SUMMARY !== undefined) {
    await appendFile(process.env.GITHUB_STEP_SUMMARY,
        `${reason}: [${release.id} ${release.version}](https://marketplace.visualstudio.com/items?itemName=${release.id}) `
        + `(${release.preRelease ? 'pre-release' : 'stable'}), source commit \`${receipt.sourceCommit}\`.\n`);
}
