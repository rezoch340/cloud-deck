import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { manifestRecipe, parseApplicationManifest } from './manifest.ts';

const text = readFileSync(
  new URL('../applications/comfyui/manifest.json', import.meta.url),
  'utf8',
);
test('ComfyUI uses the canonical lifecycle manifest and external-first resources', () => {
  const manifest = parseApplicationManifest(text);
  const application = manifestRecipe(manifest, true);
  assert.equal(application.id, 'comfyui');
  assert.equal(manifest.category, 'workflow');
  assert.equal(application.autoStart, false);
  assert.equal(application.saveDirectories.length, 3);
  assert.equal('installation' in manifest, false);
  assert.equal('usePreparedLaunch' in manifest.runtime, false);
  for (const resource of manifest.resources.files) {
    assert.deepEqual(
      resource.sources.map((source) => source.kind),
      ['download', 'drive'],
    );
  }
});
test('incomplete or unverifiable resources cannot be saved', () => {
  const manifest = JSON.parse(text);
  manifest.resources.files[0].sha256 = '';
  assert.throws(() => parseApplicationManifest(JSON.stringify(manifest)), /SHA-256/);
  assert.throws(() => parseApplicationManifest('{}'), /清单需要/);
  assert.throws(
    () => parseApplicationManifest(JSON.stringify({ ...JSON.parse(text), category: 'invalid' })),
    /分类/,
  );
});

test('a task with commands needs no downloads or Drive rules', () => {
  const manifest = parseApplicationManifest(
    readFileSync(new URL('../../examples/command-app/manifest.json', import.meta.url), 'utf8'),
  );
  const application = manifestRecipe(manifest);
  assert.equal(application.kind, 'task');
  assert.deepEqual(manifest.resources.files, []);
  assert.deepEqual(application.saveDirectories, []);
  assert.equal(application.remotePort, null);
  assert.ok(manifest.details?.includes('| 项目 | 内容 |'));
});

test('services accept HTTP, TCP and script readiness independently of a browser port', () => {
  for (const readiness of [
    { type: 'http', url: 'http://127.0.0.1:8188/ready', status: 204 },
    { type: 'tcp', host: '127.0.0.1', port: 8188 },
    { type: 'command', command: 'bash "$CLOUD_DECK_APP_DIR/ready.sh"' },
  ]) {
    const manifest = JSON.parse(text);
    manifest.runtime.readiness = readiness;
    manifest.runtime.remotePort = null;
    manifest.runtime.localPort = null;
    const parsed = parseApplicationManifest(JSON.stringify(manifest));
    assert.deepEqual(parsed.runtime.readiness, readiness);
    assert.equal(manifestRecipe(parsed).healthPath, null);
  }
});

test('invalid readiness never falls back to assuming the process is ready', () => {
  for (const readiness of [
    null,
    { type: 'unknown' },
    { type: 'command', command: ' ' },
    { type: 'tcp', host: '127.0.0.1', port: 0 },
    { type: 'http', url: 'file:///tmp/ready' },
    { type: 'command', command: 'true', timeoutSeconds: 0 },
  ]) {
    const manifest = JSON.parse(text);
    manifest.runtime.readiness = readiness;
    assert.throws(() => parseApplicationManifest(JSON.stringify(manifest)));
  }
});
