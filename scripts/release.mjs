import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { basename, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
const gh = (...args) => execFileSync('gh', args, { encoding: 'utf8' }).trim();
const json = async (path) => JSON.parse(await readFile(resolve(root, path), 'utf8'));
const save = (path, value) => writeFile(resolve(root, path), JSON.stringify(value, null, 2) + '\n');
const metadataPath = 'target/release-metadata.json';
const plainVersion = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const api = (path) => JSON.parse(gh('api', `repos/${process.env.GITHUB_REPOSITORY}/${path}`));
const pages = (path) => JSON.parse(gh('api', '--paginate', '--slurp',
  `repos/${process.env.GITHUB_REPOSITORY}/${path}?per_page=100`)).flat();

export function nextDevVersion(base, tags, sourceCommit) {
  assert.match(base, plainVersion, 'The shared base version must be a stable major.minor.patch version.');
  const [, major, minor, patch] = base.match(plainVersion);
  let nextPatch = BigInt(patch);
  let existing;
  for (const tag of tags) {
    const version = tag.name.slice(1);
    const parts = version.match(plainVersion);
    if (!tag.name.startsWith('v') || !parts || parts[1] !== major || parts[2] !== minor) continue;
    if (tag.commit.sha === sourceCommit) {
      assert.ok(!existing, `Multiple release tags point to ${sourceCommit} on the ${major}.${minor} line.`);
      existing = version;
    }
    const afterTag = BigInt(parts[3]) + 1n;
    if (afterTag > nextPatch) nextPatch = afterTag;
  }
  return existing ?? `${major}.${minor}.${nextPatch}`;
}

export function promotion(body) {
  body = body.replace(/<!--[\s\S]*?-->/g, '');
  const builds = [...body.matchAll(/^Tested dev build:[ \t]+([^\r\n]*)/gm)];
  const tag = builds[0]?.[1].trim();
  assert.ok(builds.length === 1 && tag.startsWith('v') && plainVersion.test(tag.slice(1)),
    'Name one candidate with Tested dev build: vX.Y.Z');
  assert.match(body, /^Test evidence:[ \t]+\S[^\r\n]*$/m, 'Add Test evidence: followed by the actual tests and results.');
  return tag;
}

export function checkCandidate(release, taggedCommit, expectedCommit) {
  assert.equal(release.isDraft, false, 'Tested candidate must be published.');
  assert.equal(release.isPrerelease, true, 'Tested candidate must be a pre-release.');
  assert.equal(taggedCommit, expectedCommit, 'Tested candidate must match the exact dev head. Retest after changing dev.');
}

async function baseVersion() {
  const cargo = await readFile(resolve(root, 'Cargo.toml'), 'utf8');
  const base = cargo.match(/\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1];
  assert.equal((await json('apps/desktop/package.json')).version, base, 'Frontend and Cargo versions differ.');
  assert.equal((await json('apps/desktop/src-tauri/tauri.conf.json')).version, base, 'Tauri and Cargo versions differ.');
  assert.match(base, plainVersion, 'The shared base version must be a stable major.minor.patch version.');
  return base;
}

function checkPush(branch) {
  assert.equal(process.env.GITHUB_EVENT_NAME, 'push');
  assert.equal(process.env.GITHUB_REF, `refs/heads/${branch}`);
  const sourceCommit = git('rev-parse', 'HEAD');
  assert.equal(sourceCommit, process.env.GITHUB_SHA);
  assert.equal(git('status', '--porcelain').length, 0, 'Published releases require a clean source checkout.');
  return sourceCommit;
}

async function prepare(channel) {
  assert.ok(['local', 'dev'].includes(channel), 'Build channel must be local or dev.');
  const base = await baseVersion();
  const timestamp = new Date().toISOString().replace(/\D/g, '').slice(0, 14);
  const sourceCommit = channel === 'dev' ? checkPush('dev') : git('rev-parse', 'HEAD');
  const sourceDirty = git('status', '--porcelain').length > 0;
  const version = channel === 'dev' ? nextDevVersion(base, pages('tags'), sourceCommit) : `${base}-local.${timestamp}`;
  const metadata = {
    version, channel, sourceCommit, sourceDirty, target: 'x86_64-pc-windows-msvc',
    packageName: `Goodtype-Windows-${version}`, createdAt: new Date().toISOString(),
    buildUrl: process.env.GITHUB_ACTIONS
      ? `${process.env.GITHUB_SERVER_URL}/${process.env.GITHUB_REPOSITORY}/actions/runs/${process.env.GITHUB_RUN_ID}` : null,
  };
  await mkdir(resolve(root, 'target'), { recursive: true });
  await save(metadataPath, metadata);
  // Tauri's build-time app version changes without editing canonical source manifests.
  await save('target/release-config.json', { version });
  console.log(JSON.stringify(metadata));
}

export async function sha256(path) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest('hex');
}

async function manifest() {
  const metadata = await json(metadataPath);
  assert.equal(git('rev-parse', 'HEAD'), metadata.sourceCommit, 'Source changed during packaging.');
  const dirty = git('status', '--porcelain').length > 0;
  if (metadata.channel !== 'local') assert.equal(dirty, false, 'Source changed during packaging.');
  metadata.sourceDirty ||= dirty;
  const directory = resolve(root, 'target', metadata.packageName);
  const files = {};
  for (const name of ['Goodtype.exe', 'tinymist.exe', 'goodtype-ink.exe', 'goodtype-mcp.exe', 'skill/goodtype/SKILL.md', 'ink-v1.md', 'README.txt', 'LICENSE', 'NOTICE']) {
    files[name] = await sha256(resolve(directory, name));
  }
  const archive = `${metadata.packageName}.zip`;
  const checksum = await sha256(resolve(root, 'target', archive));
  await save(`target/${metadata.packageName}.json`, { ...metadata, archive, sha256: checksum, files });
  await writeFile(resolve(root, 'target', `${metadata.packageName}.sha256`), `${checksum}  ${archive}\n`);
}

function checkPromotionSource(pr) {
  assert.equal(pr.head.repo?.full_name, process.env.GITHUB_REPOSITORY, 'Promote the dev branch from this repository.');
  assert.equal(pr.head.ref, 'dev', 'Only dev may be merged into main. Send feature PRs to dev.');
}

function taggedCommit(tag) {
  let { object } = api(`git/ref/tags/${tag}`);
  while (object.type === 'tag') ({ object } = api(`git/tags/${object.sha}`));
  assert.equal(object.type, 'commit', `${tag} must point to a commit.`);
  return object.sha;
}

function releaseByTag(tag) {
  return JSON.parse(gh('release', 'view', tag, '--repo', process.env.GITHUB_REPOSITORY,
    '--json', 'isDraft,isPrerelease,tagName'));
}

async function policy() {
  const { pull_request: pr } = JSON.parse(await readFile(process.env.GITHUB_EVENT_PATH, 'utf8'));
  if (!pr) return;
  assert.ok(['dev', 'main'].includes(pr.base.ref), 'Pull requests must target dev or main.');
  if (pr.base.ref !== 'main') return;
  checkPromotionSource(pr);
  const tag = promotion(pr.body ?? '');
  checkCandidate(releaseByTag(tag), taggedCommit(tag), pr.head.sha);
  checkDevCI(pr.head.sha);
}

function checkDevCI(commit) {
  const runs = JSON.parse(gh('run', 'list', '--repo', process.env.GITHUB_REPOSITORY, '--workflow', 'ci.yml', '--branch', 'dev',
    '--commit', commit, '--event', 'push', '--status', 'success', '--json', 'databaseId'));
  assert.ok(runs.length > 0, 'Wait for the candidate build and all CI checks to pass.');
}

function promote() {
  const sourceCommit = checkPush('main');
  const prs = pages(`commits/${sourceCommit}/pulls`)
    .filter((pr) => pr.merged_at && pr.merge_commit_sha === sourceCommit && pr.base.ref === 'main');
  assert.equal(prs.length, 1, 'Expected exactly one merged promotion PR for this main commit.');
  const [pr] = prs;
  checkPromotionSource(pr);
  const tag = promotion(pr.body ?? '');
  const commit = taggedCommit(tag);
  assert.equal(commit, pr.head.sha, 'Tested candidate must match the merged dev head.');
  assert.equal(git('rev-parse', 'HEAD^{tree}'), api(`git/commits/${commit}`).tree.sha,
    'The main source tree differs from the tested candidate.');
  const release = releaseByTag(tag);
  assert.equal(release.isDraft, false, 'Tested candidate must be published.');
  if (!release.isPrerelease) {
    console.log(`${tag} is already stable.`);
    return;
  }
  checkDevCI(commit);
  gh('release', 'edit', tag, '--repo', process.env.GITHUB_REPOSITORY, '--prerelease=false', '--latest');
}

async function publish(directory) {
  const sourceCommit = checkPush('dev');
  const { readdir } = await import('node:fs/promises');
  const names = await readdir(directory);
  const manifests = names.filter((name) => /^Goodtype-Windows-.*\.json$/.test(name));
  assert.equal(manifests.length, 1);
  const metadata = JSON.parse(await readFile(resolve(directory, manifests[0]), 'utf8'));
  assert.equal(metadata.sourceCommit, sourceCommit);
  assert.equal(metadata.sourceDirty, false);
  assert.equal(metadata.channel, 'dev');
  assert.match(metadata.version, plainVersion);
  assert.equal(basename(metadata.archive), metadata.archive);
  assert.equal(await sha256(resolve(directory, metadata.archive)), metadata.sha256);
  const tag = `v${metadata.version}`;
  const repository = process.env.GITHUB_REPOSITORY;
  const existingTag = pages('tags').find((item) => item.name === tag);
  if (existingTag) {
    assert.equal(taggedCommit(tag), sourceCommit, `${tag} already points to a different commit.`);
  }
  const existingRelease = pages('releases').find((item) => item.tag_name === tag);
  if (existingRelease) {
    assert.ok(existingTag && !existingRelease.draft, `${tag} has an inconsistent or draft release. Published versions are never replaced.`);
    console.log(`${tag} is already published for ${sourceCommit}.`);
    return;
  }
  if (!existingTag) {
    // Creating the ref is atomic. If the name was taken since the listing, GitHub rejects it.
    gh('api', '--method', 'POST', `repos/${repository}/git/refs`, '-f', `ref=refs/tags/${tag}`, '-f', `sha=${sourceCommit}`);
  }
  const args = ['release', 'create', tag, ...names.map((name) => resolve(directory, name)),
    '--repo', repository, '--verify-tag', '--title', `Goodtype ${tag} - Pre-alpha`,
    '--prerelease', '--latest=false',
    '--notes', `Source: ${metadata.sourceCommit}\nBuild: ${metadata.buildUrl}\n\nWindows x64 portable ZIP. Extract the entire folder. WebView2 is required. This build is unsigned; physical pen/touch validation remains separate. See the included README and SHA-256 metadata.`];
  gh(...args);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [command, argument] = process.argv.slice(2);
  try {
    if (command === 'prepare') await prepare(argument);
    else if (command === 'manifest') await manifest();
    else if (command === 'policy') await policy();
    else if (command === 'publish') await publish(resolve(argument));
    else if (command === 'promote') promote();
    else throw new Error('Usage: node scripts/release.mjs prepare local|dev | manifest | policy | publish DIRECTORY | promote');
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
