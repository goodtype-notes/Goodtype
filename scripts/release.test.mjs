import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { checkCandidate, nextDevVersion, promotion, sha256 } from './release.mjs';

const tag = (name, sha = 'previous') => ({ name, commit: { sha } });

test('dev versions use the base floor and advance the highest plain tag on the same line', () => {
  const tags = [tag('v0.1.0'), tag('v0.3.1-dev.55.1', 'current'), tag('v1.3.100'), tag('v0.3.99-local.20261027143000')];
  assert.equal(nextDevVersion('0.3.1', tags, 'current'), '0.3.1');
  tags.push(tag('v0.3.1'), tag('v0.3.10'), tag('v0.3.2'));
  assert.equal(nextDevVersion('0.3.1', tags, 'current'), '0.3.11');
  assert.equal(nextDevVersion('0.3.20', tags, 'current'), '0.3.20');
  assert.equal(nextDevVersion('0.4.0', tags, 'current'), '0.4.0');
});

test('a retry reuses the tag for its commit even after a newer candidate', () => {
  const tags = [tag('v0.3.2'), tag('v0.3.1', 'current')];
  assert.equal(nextDevVersion('0.3.1', tags, 'current'), '0.3.1');
  assert.throws(() => nextDevVersion('0.3.1', [...tags, tag('v0.3.3', 'current')], 'current'), /Multiple release tags/);
});

test('base versions reject paths and invalid SemVer', () => {
  for (const base of ['../0.1.0', '01.1.0', '0.1', '0.1.0-beta', '0.1.0\n']) {
    assert.throws(() => nextDevVersion(base, [], 'current'));
  }
});

test('promotion requires the correct versioned candidate and recorded test evidence', () => {
  const body = 'Tested dev build: v0.3.12\r\nTest evidence: ZIP hashes and relocated decoder passed.';
  assert.equal(promotion(body), 'v0.3.12');
  assert.equal(promotion(`<!-- Tested dev build: v0.x.y -->\n${body}`), 'v0.3.12');
  for (const invalid of ['', body.replace('v0.3.12', 'v0.3.12-dev.55.1'), body.replace('v0.3.12', 'v0.3.012'),
    body.replace('build: ', 'build:'), body.split('\r\n')[0], `${body}\nTested dev build: v0.3.13`, `<!-- ${body} -->`,
    'Tested dev build: v0.3.12\nTest evidence: \n\nUnrelated text']) {
    assert.throws(() => promotion(invalid));
  }
});

test('checksums identify content changes in release assets', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'goodtype-release-test-'));
  try {
    const path = join(directory, 'asset');
    await writeFile(path, 'abc');
    assert.equal(await sha256(path), 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
    await writeFile(path, 'abd');
    assert.notEqual(await sha256(path), 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
  } finally {
    await rm(directory, { recursive: true });
  }
});

test('changing dev after testing invalidates promotion; drafts and stable tags are not candidates', () => {
  const release = { isDraft: false, isPrerelease: true };
  const commit = 'a'.repeat(40);
  checkCandidate(release, commit, commit);
  assert.throws(() => checkCandidate(release, commit, 'b'.repeat(40)), /exact dev head/);
  assert.throws(() => checkCandidate({ ...release, isDraft: true }, commit, commit), /published/);
  assert.throws(() => checkCandidate({ ...release, isPrerelease: false }, commit, commit), /pre-release/);
});
