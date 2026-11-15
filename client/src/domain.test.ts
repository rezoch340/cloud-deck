import test from 'node:test';
import assert from 'node:assert/strict';
import {
  cloudLabels,
  emptyObservation,
  instanceAction,
  instanceRegion,
  mayLeaveAccount,
  newDraft,
  updateSummary,
  validateRecipe,
} from './domain.ts';
import type { Recipe, Snapshot } from './domain.ts';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { manifestRecipe, parseApplicationManifest } from './manifest.ts';
import { normalizeReleaseVersion, syncClientVersion } from '../scripts/sync-version.mjs';

const builtin: Recipe = manifestRecipe(
  parseApplicationManifest(
    readFileSync(new URL('../applications/comfyui/manifest.json', import.meta.url), 'utf8'),
  ),
  true,
);

const account = {
  subject: 'test',
  name: 'Test',
  email: 'test@example.com',
  status: 'connected' as const,
};
const snapshot = (): Snapshot => ({
  apps: [builtin],
  account,
  observation: emptyObservation(),
  events: [],
  mode: 'preview',
  dataDirectory: 'test',
  version: '0.1.0',
  update: {
    status: 'idle',
    currentVersion: '0.1.0',
    releaseUrl: 'https://github.com/rezoch340/cloud-deck/releases',
    canSelfUpdate: false,
    installMode: 'macos_bundle',
  },
  runtime: {
    connection: 'not_connected',
    synchronization: 'unobserved',
    storage: {},
    applications: { comfyui: 'unknown' },
    applicationUrls: {},
    endpoint: null,
    phase: 'idle',
    message: '',
    needsDriveConsent: false,
  },
});

test('boot is offered only after confirming no instance; connected instances lead to applications', () => {
  const state = snapshot();
  assert.equal(instanceAction(state).kind, 'refresh');
  state.observation.status = 'current';
  assert.equal(instanceAction(state).kind, 'start');
  state.observation.instances = [{ id: 'allocated-l4', accelerator: 'L4' }];
  assert.deepEqual(instanceAction(state), {
    kind: 'connect',
    label: '连接已有实例',
    endpoint: 'allocated-l4',
  });
  state.runtime.endpoint = 'allocated-l4';
  state.runtime.connection = 'connected';
  assert.deepEqual(instanceAction(state), { kind: 'applications', label: '部署应用' });
  state.runtime.connection = 'disconnected';
  assert.deepEqual(instanceAction(state), {
    kind: 'connect',
    label: '重新连接',
    endpoint: 'allocated-l4',
  });
});

test('stale observations and replaced instances never send the user through an implicit boot', () => {
  const state = snapshot();
  state.runtime.endpoint = 'previous-instance';
  state.observation.instances = [{ id: 'current-instance', accelerator: 'L4' }];
  for (const status of ['cached', 'error', 'unobserved'] as const) {
    state.observation.status = status;
    assert.equal(instanceAction(state).kind, 'refresh');
  }
  state.observation.status = 'current';
  state.runtime.connection = 'connected';
  assert.deepEqual(instanceAction(state), {
    kind: 'connect',
    label: '连接已有实例',
    endpoint: 'current-instance',
  });
  state.observation.instances.push({ id: 'another-instance', accelerator: 'T4' });
  assert.equal(instanceAction(state).kind, 'unavailable');
});

test('unknown, cached and failed observations never confirm zero billing', () => {
  for (const status of ['unobserved', 'cached', 'error'] as const) {
    const state = snapshot();
    state.observation = { ...state.observation, status, hourlyRate: 0, balance: 0 };
    assert.equal(cloudLabels(state.observation).confirmedOff, false);
    assert.equal(mayLeaveAccount(state), false);
    assert.equal(state.runtime.synchronization, 'unobserved');
    assert.equal(state.runtime.applications.comfyui, 'unknown');
  }
});
test('leaving an account requires both a current empty instance list and zero rate', () => {
  const state = snapshot();
  state.observation = { ...state.observation, status: 'current', hourlyRate: 0 };
  assert.equal(mayLeaveAccount(state), true);
  state.observation.instances = [{ id: 'runtime', accelerator: 'G4' }];
  assert.equal(mayLeaveAccount(state), false);
  state.observation.instances = [];
  state.observation.hourlyRate = 1;
  assert.equal(mayLeaveAccount(state), false);
});
test('allocated regions distinguish current, missing and historical observations', () => {
  const observation = {
    ...emptyObservation(),
    status: 'current' as const,
    hourlyRate: 1.15,
    instances: [
      {
        id: 't4',
        accelerator: 'T4',
        region: 'asia-southeast1',
        regionLabel: '新加坡 · asia-southeast1',
      },
    ],
  };
  assert.deepEqual(instanceRegion(observation), {
    label: '分配地区',
    value: '新加坡 · asia-southeast1',
    historical: false,
  });
  assert.equal(instanceRegion({ ...observation, status: 'error' }).label, '上次观测地区');
  assert.equal(
    instanceRegion({ ...observation, instances: [{ id: 'unknown', accelerator: 'T4' }] }).value,
    '地区待确认',
  );
  const released = {
    ...observation,
    instances: [],
    lastInstance: observation.instances[0],
    hourlyRate: 0,
  };
  assert.equal(instanceRegion(released).label, '上次分配地区');
  assert.equal(instanceRegion(released).value, '新加坡 · asia-southeast1');
  assert.equal(cloudLabels(released).confirmedOff, true);
});

test('generic service configurations protect names, ports and built-in recipes', () => {
  const draft = { ...newDraft(), name: 'My service' };
  assert.equal(validateRecipe(draft, [builtin]), null);
  assert.match(
    validateRecipe({ ...draft, remotePort: builtin.remotePort }, [builtin]) || '',
    /端口/,
  );
  assert.match(validateRecipe({ ...draft, localPort: builtin.localPort }, [builtin]) || '', /端口/);
  assert.match(validateRecipe({ ...draft, name: 'comfyUI' }, [builtin]) || '', /同名/);
  assert.match(validateRecipe({ ...draft, id: 'comfyui' }, [builtin]) || '', /内置/);
  for (const port of [0, -1, 65536, 1.5, NaN])
    assert.notEqual(validateRecipe({ ...draft, remotePort: port }, [builtin]), null);
});
test('editing a configuration does not collide with its own reserved ports', () => {
  const draft = { ...newDraft(), id: 'custom', name: 'My service' };
  const existing: Recipe = { ...draft, id: 'custom', builtin: false };
  assert.equal(validateRecipe(draft, [builtin, existing]), null);
});
test('one-shot tasks do not need a web port or ComfyUI health endpoint', () => {
  const draft = { ...newDraft('task'), name: 'Batch processing' };
  assert.equal(draft.remotePort, null);
  assert.equal(draft.healthPath, null);
  assert.equal(validateRecipe(draft, [builtin]), null);
});
test('configuration input rejects directory traversal and invalid command bytes', () => {
  const draft = { ...newDraft(), name: 'My service' };
  assert.match(validateRecipe({ ...draft, command: 'python\0app.py' }, [builtin]) || '', /命令/);
  assert.match(
    validateRecipe({ ...draft, workingDirectory: '/workspace/../etc' }, [builtin]) || '',
    /目录/,
  );
  assert.match(
    validateRecipe({ ...draft, saveDirectories: ['relative/output'] }, [builtin]) || '',
    /目录/,
  );
  assert.match(
    validateRecipe({ ...draft, healthPath: '//other-host/path' }, [builtin]) || '',
    /健康检查/,
  );
});

test('update summary distinguishes self-updatable Windows NSIS builds from manual download bundles', () => {
  const baseUpdate = snapshot().update;
  assert.equal(updateSummary(baseUpdate).badge.label, '尚未检查');
  assert.match(updateSummary(baseUpdate).modeLabel, /macOS/);

  const nsisAvailable = updateSummary({
    ...baseUpdate,
    status: 'available',
    latestVersion: '0.2.0',
    canSelfUpdate: true,
    installMode: 'windows_nsis',
  });
  assert.equal(nsisAvailable.badge.label, '发现新版本 v0.2.0');
  assert.equal(nsisAvailable.badge.tone, 'accent');
  assert.match(nsisAvailable.modeLabel, /自动覆盖更新/);

  assert.equal(updateSummary({ ...baseUpdate, status: 'up_to_date' }).badge.label, '已是最新版本');
  assert.equal(
    updateSummary({ ...baseUpdate, status: 'no_release' }).badge.label,
    '暂无已发布版本',
  );
});

test('sync-version normalizes release tags and keeps all four client manifests consistent', () => {
  assert.equal(normalizeReleaseVersion('refs/tags/v1.4.2'), '1.4.2');
  assert.equal(normalizeReleaseVersion('v0.2.0-rc.1'), '0.2.0-rc.1');
  assert.throws(() => normalizeReleaseVersion('invalid-tag'), /版本号格式无效/);

  const tempBase = process.env.TMPDIR || os.tmpdir();
  const fixtureRoot = mkdtempSync(path.join(tempBase, 'cloud-deck-version-test-'));
  try {
    mkdirSync(path.join(fixtureRoot, 'src-tauri'), { recursive: true });
    writeFileSync(
      path.join(fixtureRoot, 'package.json'),
      JSON.stringify({ name: 'cloud-deck-client', version: '0.1.0' }, null, 2),
    );
    writeFileSync(
      path.join(fixtureRoot, 'src-tauri/tauri.conf.json'),
      JSON.stringify({ productName: 'Cloud Deck', version: '0.1.0' }, null, 2),
    );
    writeFileSync(
      path.join(fixtureRoot, 'src-tauri/Cargo.toml'),
      '[package]\nname = "cloud-deck"\nversion = "0.1.0"\nedition = "2021"\n',
    );
    writeFileSync(
      path.join(fixtureRoot, 'src-tauri/Cargo.lock'),
      '[[package]]\nname = "cloud-deck"\nversion = "0.1.0"\n',
    );

    const synced = syncClientVersion(fixtureRoot, 'v0.3.1');
    assert.equal(synced, '0.3.1');
    assert.equal(syncClientVersion(fixtureRoot, '0.3.1', true), '0.3.1');
    assert.throws(() => syncClientVersion(fixtureRoot, '0.3.2', true), /不一致/);
  } finally {
    rmSync(fixtureRoot, { recursive: true, force: true });
  }
});
