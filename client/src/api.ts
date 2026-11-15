import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import comfyManifest from '../applications/comfyui/manifest.json';
import packageJson from '../package.json';
import { manifestRecipe, parseApplicationManifest } from './manifest.ts';
import type { ApplicationManifest } from './manifest.ts';
import { emptyObservation, validateRecipe } from './domain.ts';
import type {
  ApplicationLog,
  AuthProgress,
  Recipe,
  RecipeDraft,
  Snapshot,
  UpdateStatus,
} from './domain.ts';

export const desktop = isTauri();
const key = 'cloud-deck-browser-preview-v1';
export const builtinRecipe = manifestRecipe(
  parseApplicationManifest(JSON.stringify(comfyManifest)),
  true,
);

/** 生成浏览器预览模式使用的默认客户端更新状态。 */
export function defaultUpdateStatus(currentVersion = packageJson.version): UpdateStatus {
  return {
    status: 'idle',
    currentVersion,
    latestVersion: null,
    releaseName: null,
    releaseNotes: null,
    releaseUrl: 'https://github.com/rezoch340/cloud-deck/releases',
    publishedAt: null,
    downloadUrl: null,
    assetName: null,
    assetSize: null,
    checksumUrl: null,
    canSelfUpdate: false,
    installMode: 'macos_bundle',
    checkedAt: null,
    error: null,
  };
}

function preview(): Snapshot {
  const raw = localStorage.getItem(key);
  let apps: Recipe[] = [];
  let events: Snapshot['events'] = [];
  if (raw) {
    let saved: unknown;
    try {
      saved = JSON.parse(raw);
    } catch {
      throw new Error('浏览器预览配置无法读取，原记录未覆盖。');
    }
    if (!saved || typeof saved !== 'object' || !('apps' in saved) || !Array.isArray(saved.apps))
      throw new Error('浏览器预览配置无效，原记录未覆盖。');
    for (const item of saved.apps) {
      if (
        !item ||
        typeof item.id !== 'string' ||
        item.builtin ||
        validateRecipe(item, [builtinRecipe, ...apps])
      )
        throw new Error('浏览器预览应用记录无效，原记录未覆盖。');
      apps.push({ ...item, builtin: false });
    }
    if ('events' in saved && Array.isArray(saved.events))
      events = saved.events
        .filter((event) => typeof event?.message === 'string' && typeof event?.time === 'number')
        .slice(-500);
  }
  return {
    apps: [builtinRecipe, ...apps],
    account: null,
    observation: emptyObservation(),
    events,
    runtime: {
      connection: 'not_connected',
      synchronization: 'unobserved',
      storage: {},
      applications: Object.fromEntries(
        [builtinRecipe, ...apps].map((application) => [application.id, 'unknown']),
      ),
      applicationUrls: {},
      endpoint: null,
      phase: 'idle',
      message: '浏览器预览不连接真实云端。',
      needsDriveConsent: false,
    },
    mode: 'preview',
    dataDirectory: '仅保存在当前浏览器的预览数据中',
    version: packageJson.version,
    update: defaultUpdateStatus(),
  };
}
function persistPreview(snapshot: Snapshot) {
  localStorage.setItem(
    key,
    JSON.stringify({
      apps: snapshot.apps.filter((application) => !application.builtin),
      events: snapshot.events.slice(-500),
    }),
  );
  return snapshot;
}
function nativeOnly(): never {
  throw new Error('请在桌面客户端执行此操作。浏览器预览不会访问真实账号或云端。');
}
export async function getWorkspace(): Promise<Snapshot> {
  return desktop ? invoke<Snapshot>('get_workspace') : preview();
}
export async function saveApplication(draft: RecipeDraft) {
  if (desktop) return invoke<Snapshot>('save_application', { draft });
  const snapshot = preview();
  const error = validateRecipe(draft, snapshot.apps);
  if (error) throw new Error(error);
  if (draft.id && !snapshot.apps.some((application) => application.id === draft.id))
    throw new Error('要修改的应用不存在。');
  const applicationId = draft.id || `app-${crypto.randomUUID()}`;
  const next: Recipe = {
    ...draft,
    id: applicationId,
    name: draft.name.trim(),
    command: draft.command.trim(),
    builtin: false,
    remotePort: draft.kind === 'service' ? draft.remotePort : null,
    localPort: draft.kind === 'service' ? draft.localPort : null,
    healthPath: draft.kind === 'service' ? draft.healthPath : null,
  };
  snapshot.apps = [
    ...snapshot.apps.filter((application) => application.id !== applicationId),
    next,
  ];
  snapshot.events.push({
    time: Date.now(),
    level: 'info',
    category: 'application',
    message: `已保存应用配置：${next.name}`,
  });
  return persistPreview(snapshot);
}
export async function saveApplicationManifest(manifest: ApplicationManifest) {
  if (desktop) return invoke<Snapshot>('save_application_manifest', { manifest });
  const snapshot = preview();
  const next = manifestRecipe(parseApplicationManifest(JSON.stringify(manifest)));
  const error = validateRecipe(next, snapshot.apps);
  if (error) throw new Error(error);
  snapshot.apps = [...snapshot.apps.filter((application) => application.id !== next.id), next];
  snapshot.events.push({
    time: Date.now(),
    level: 'info',
    category: 'application',
    message: `已保存应用清单：${next.name}`,
  });
  return persistPreview(snapshot);
}
export async function removeApplication(applicationId: string) {
  if (desktop) return invoke<Snapshot>('remove_application', { id: applicationId });
  const snapshot = preview();
  const app = snapshot.apps.find((application) => application.id === applicationId);
  if (!app || app.builtin) throw new Error('内置应用不能移除。');
  snapshot.apps = snapshot.apps.filter((application) => application.id !== applicationId);
  snapshot.events.push({
    time: Date.now(),
    level: 'info',
    category: 'application',
    message: `已移除应用配置：${app.name}；未删除云端文件。`,
  });
  return persistPreview(snapshot);
}
export const signIn = () => (desktop ? invoke<Snapshot>('sign_in') : nativeOnly());
export const cancelSignIn = () => (desktop ? invoke<void>('cancel_sign_in') : nativeOnly());
export const reopenSignIn = () => (desktop ? invoke<void>('reopen_sign_in') : nativeOnly());
export const refreshAccount = async () => {
  if (!desktop) return nativeOnly();
  const snapshot = await invoke<Snapshot>('refresh_account');
  return snapshot.runtime.endpoint ? invoke<Snapshot>('refresh_cloud') : snapshot;
};
export const startInstance = (accelerator: string) =>
  desktop ? invoke<Snapshot>('start_instance', { accelerator }) : nativeOnly();
export const connectInstance = (endpoint: string) =>
  desktop ? invoke<Snapshot>('connect_instance', { endpoint }) : nativeOnly();
export const refreshCloud = () => (desktop ? invoke<Snapshot>('refresh_cloud') : nativeOnly());
export const startApplication = (applicationId: string) =>
  desktop ? invoke<Snapshot>('start_application', { id: applicationId }) : nativeOnly();
export const stopApplication = (applicationId: string) =>
  desktop ? invoke<Snapshot>('stop_application', { id: applicationId }) : nativeOnly();
export const applicationLogs = (applicationId: string) =>
  desktop ? invoke<ApplicationLog>('application_logs', { id: applicationId }) : nativeOnly();
export const openApplication = (applicationId: string) =>
  desktop ? invoke<void>('open_application', { id: applicationId }) : nativeOnly();
export const openDriveConsent = () => (desktop ? invoke<void>('open_drive_consent') : nativeOnly());
export const synchronizeFiles = () =>
  desktop ? invoke<Snapshot>('synchronize_files') : nativeOnly();
export const shutdownCloud = () => (desktop ? invoke<Snapshot>('shutdown_cloud') : nativeOnly());
export const releaseEmptyInstance = () =>
  desktop ? invoke<Snapshot>('release_empty_instance') : nativeOnly();
export const signOut = () => (desktop ? invoke<Snapshot>('sign_out') : nativeOnly());
export const checkUpdate = () => (desktop ? invoke<Snapshot>('check_update') : nativeOnly());
export const installUpdate = () => (desktop ? invoke<Snapshot>('install_update') : nativeOnly());
export const openReleasePage = () => (desktop ? invoke<void>('open_release_page') : nativeOnly());
export async function onAuthProgress(callback: (progress: AuthProgress) => void) {
  if (!desktop) return () => {};
  return listen<AuthProgress>('auth-progress', (event) => callback(event.payload));
}
