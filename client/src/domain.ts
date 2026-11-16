import type { ApplicationManifest } from './manifest.ts';
import { resolveReadiness, type Readiness } from './readiness.ts';

export type AppKind = 'service' | 'task';
export interface Recipe {
  manifest?: ApplicationManifest | null;
  id: string;
  name: string;
  description: string;
  kind: AppKind;
  command: string;
  prepareCommand?: string | null;
  environment?: Record<string, string>;
  workingDirectory: string;
  remotePort: number | null;
  localPort: number | null;
  healthPath: string | null;
  readiness?: Readiness | null;
  saveDirectories: string[];
  storageDirectory: string;
  autoStart: boolean;
  builtin: boolean;
}
export type RecipeDraft = Omit<Recipe, 'id' | 'builtin' | 'manifest'> & {
  id: string | null;
};
export interface Account {
  subject: string;
  email: string;
  name: string;
  status: 'saved' | 'connected';
}
export interface Instance {
  id: string;
  accelerator: string | null;
  region?: string | null;
  regionLabel?: string | null;
}
export interface Observation {
  status: 'unobserved' | 'current' | 'cached' | 'error';
  observedAt: number | null;
  balance: number | null;
  hourlyRate: number | null;
  eligibleGpus: string[];
  instances: Instance[];
  lastInstance: Instance | null;
  error: string | null;
}
export interface Activity {
  time: number;
  level: 'info' | 'error';
  category: string;
  message: string;
}
export interface ApplicationLog {
  text: string;
  truncated: boolean;
  state: string;
  stage: string;
  error?: string | null;
  observedAt: number;
  progress?: {
    stage: string;
    message: string;
    completedBytes?: number;
    totalBytes?: number;
    updatedAt?: number;
  } | null;
}
/** 客户端版本检查与自更新状态。 */
export interface UpdateStatus {
  status:
    | 'idle'
    | 'checking'
    | 'up_to_date'
    | 'available'
    | 'no_release'
    | 'installing'
    | 'error'
    | string;
  currentVersion: string;
  latestVersion?: string | null;
  releaseName?: string | null;
  releaseNotes?: string | null;
  releaseUrl: string;
  publishedAt?: string | null;
  downloadUrl?: string | null;
  assetName?: string | null;
  assetSize?: number | null;
  checksumUrl?: string | null;
  canSelfUpdate: boolean;
  installMode: 'windows_nsis' | 'windows_portable' | 'macos_bundle' | string;
  checkedAt?: number | null;
  error?: string | null;
}
export interface Snapshot {
  apps: Recipe[];
  account: Account | null;
  observation: Observation;
  events: Activity[];
  lastSaveVerification?: {
    accountSubject: string;
    endpoint: string;
    storage: Snapshot['runtime']['storage'];
  } | null;
  runtime: {
    heartbeat?: {
      status: string;
      lastAttemptAt?: number;
      lastSuccessAt?: number;
      google?: string;
      kernel?: string;
      owner?: string;
      successCount?: number;
      error?: string | null;
    };
    connection: 'not_connected' | 'connected' | 'disconnected';
    synchronization: 'unobserved' | 'pending' | 'syncing' | 'verified' | 'failed';
    storage: {
      observedAt?: number;
      method?: string;
      receiptId?: string;
      fileCount?: number;
      totalBytes?: number;
      error?: string;
      files?: { applicationId: string; path: string; size: number; sha256: string; md5: string }[];
    };
    applications: Record<string, 'unknown' | 'preparing' | 'running' | 'stopped' | 'failed'>;
    applicationUrls: Record<string, string>;
    endpoint: string | null;
    phase: string;
    message: string;
    needsDriveConsent: boolean;
  };
  dataDirectory: string;
  mode: 'desktop' | 'preview';
  version: string;
  update: UpdateStatus;
}
export interface AuthProgress {
  phase: 'preparing' | 'waiting' | 'verifying' | 'checking';
  message: string;
}
/** 将客户端版本检查结果转换为界面展示徽章与安装形态说明。 */
export function updateSummary(update: UpdateStatus) {
  const modeLabel =
    update.installMode === 'windows_nsis'
      ? 'Windows 安装版（支持校验后自动覆盖更新）'
      : update.installMode === 'windows_portable'
        ? 'Windows 便携版（前往 GitHub Release 下载替换）'
        : update.installMode === 'macos_bundle'
          ? 'macOS 应用包（前往 GitHub Release 下载替换）'
          : '桌面客户端';
  const badge =
    update.status === 'available'
      ? ({
          label: update.latestVersion ? `发现新版本 v${update.latestVersion}` : '发现新版本',
          tone: 'accent',
        } as const)
      : update.status === 'up_to_date'
        ? ({ label: '已是最新版本', tone: 'good' } as const)
        : update.status === 'no_release'
          ? ({ label: '暂无已发布版本', tone: 'neutral' } as const)
          : update.status === 'checking'
            ? ({ label: '正在检查…', tone: 'neutral' } as const)
            : update.status === 'installing'
              ? ({ label: '正在校验并安装…', tone: 'accent' } as const)
              : update.status === 'error'
                ? ({ label: '检查未完成', tone: 'bad' } as const)
                : ({ label: '尚未检查', tone: 'neutral' } as const);
  return { modeLabel, badge };
}
export const emptyObservation = (): Observation => ({
  status: 'unobserved',
  observedAt: null,
  balance: null,
  hourlyRate: null,
  eligibleGpus: [],
  instances: [],
  lastInstance: null,
  error: null,
});

export function cloudLabels(observation: Observation) {
  if (observation.status !== 'current')
    return {
      instance: '状态待确认',
      billing: '计费待确认',
      tone: 'warn',
      confirmedOff: false,
    } as const;
  const active = observation.instances.length > 0;
  const off = !active && observation.hourlyRate === 0;
  return {
    instance: active ? '实例运行中' : '未发现实例',
    billing: off
      ? '已确认当前无计费'
      : observation.hourlyRate === null
        ? '计费待确认'
        : '实例计费中',
    tone: active ? 'good' : off ? 'neutral' : 'warn',
    confirmedOff: off,
  } as const;
}

export type InstanceAction =
  | { kind: 'refresh'; label: string }
  | { kind: 'start'; label: string }
  | { kind: 'connect'; label: string; endpoint: string }
  | { kind: 'applications'; label: string }
  | { kind: 'unavailable'; label: string };

export function instanceAction(snapshot: Snapshot): InstanceAction {
  const { observation, runtime } = snapshot;
  const connected = runtime.connection === 'connected' && runtime.endpoint !== null;
  if (
    connected &&
    (observation.status !== 'current' ||
      observation.instances.some((instance) => instance.id === runtime.endpoint))
  ) {
    return { kind: 'applications', label: '部署应用' };
  }
  if (observation.status !== 'current') return { kind: 'refresh', label: '刷新实例状态' };
  if (observation.instances.length === 0) return { kind: 'start', label: '启动实例' };
  if (observation.instances.length !== 1)
    return { kind: 'unavailable', label: '请确认要使用的实例' };
  const endpoint = observation.instances[0].id;
  return {
    kind: 'connect',
    label: runtime.endpoint === endpoint ? '重新连接' : '连接已有实例',
    endpoint,
  };
}

export function instanceRegion(observation: Observation) {
  const active = observation.instances[0];
  const instance = active || observation.lastInstance;
  return {
    label: active
      ? observation.status === 'current'
        ? '分配地区'
        : '上次观测地区'
      : instance
        ? '上次分配地区'
        : '分配地区',
    value: instance
      ? instance.regionLabel || instance.region || '地区待确认'
      : observation.status === 'current'
        ? '尚未分配'
        : '尚未查询',
    historical: !active || observation.status !== 'current',
  };
}

export function mayLeaveAccount(snapshot: Snapshot) {
  return !snapshot.account || cloudLabels(snapshot.observation).confirmedOff;
}
export const bytes = (value: number) => {
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index++;
  }
  return `${value.toFixed(index ? 2 : 0)} ${units[index]}`;
};
export const observedTime = (value: number | null) =>
  value ? new Date(value).toLocaleString('zh-CN', { hour12: false }) : '尚未观测';
const directory = (value: unknown): value is string =>
  typeof value === 'string' &&
  value.startsWith('/') &&
  value.length <= 512 &&
  !/[\0\r\n]/.test(value) &&
  !value.split('/').includes('..');

export function validateRecipe(draft: RecipeDraft, apps: Recipe[]): string | null {
  if (
    typeof draft.name !== 'string' ||
    !draft.name.trim() ||
    [...draft.name.trim()].length > 60 ||
    /[\0\r\n]/.test(draft.name)
  )
    return '应用名称应为 1–60 个字符。';
  if (
    draft.id === 'comfyui' ||
    apps.some((application) => application.id === draft.id && application.builtin)
  )
    return '内置应用保留项目固定配置，不能覆盖。';
  if (
    apps.some(
      (application) =>
        application.id !== draft.id &&
        application.name.toLowerCase() === draft.name.trim().toLowerCase(),
    )
  )
    return '已有同名应用，请换一个名称。';
  if (!['service', 'task'].includes(draft.kind)) return '不支持的应用类型。';
  if (
    typeof draft.command !== 'string' ||
    !draft.command.trim() ||
    new TextEncoder().encode(draft.command).length > 4000 ||
    draft.command.includes('\0')
  )
    return '请填写有效的启动命令（最多 4000 字节）。';
  if (
    draft.prepareCommand != null &&
    (typeof draft.prepareCommand !== 'string' ||
      new TextEncoder().encode(draft.prepareCommand).length > 4000 ||
      draft.prepareCommand.includes('\0'))
  )
    return '准备命令无效。';
  if (
    draft.environment &&
    (typeof draft.environment !== 'object' ||
      Array.isArray(draft.environment) ||
      Object.keys(draft.environment).length > 64 ||
      Object.entries(draft.environment).some(
        ([key, value]) =>
          !key ||
          /[=\0]/.test(key) ||
          typeof value !== 'string' ||
          value.includes('\0') ||
          value.length > 8192,
      ))
  )
    return '环境变量无效。';
  if (
    typeof draft.description !== 'string' ||
    new TextEncoder().encode(draft.description).length > 1000 ||
    !directory(draft.workingDirectory) ||
    !directory(draft.storageDirectory) ||
    !Array.isArray(draft.saveDirectories) ||
    draft.saveDirectories.length > 32 ||
    !draft.saveDirectories.every(directory)
  )
    return '目录必须是云端 Linux 的绝对路径，不能包含上级目录跳转。';
  if (draft.kind === 'service') {
    if (
      ![draft.remotePort, draft.localPort].every(
        (port) => port === null || (Number.isInteger(port) && port >= 1 && port <= 65535),
      ) ||
      (draft.localPort !== null && draft.remotePort === null)
    )
      return '服务需要有效的远端端口与本机端口（1–65535）。';
    if (
      apps.some(
        (application) =>
          application.id !== draft.id &&
          ((draft.localPort !== null && application.localPort === draft.localPort) ||
            (draft.remotePort !== null && application.remotePort === draft.remotePort)),
      )
    )
      return '端口已被其他应用配置使用，请更换。';
  }
  try {
    resolveReadiness(draft, draft.kind);
  } catch (error) {
    return String(error instanceof Error ? error.message : error);
  }
  return null;
}

export function newDraft(kind: AppKind = 'service'): RecipeDraft {
  return {
    id: null,
    name: '',
    description: '',
    kind,
    command: kind === 'task' ? 'python main.py' : 'python app.py --port 7860',
    workingDirectory: '/workspace/my-app',
    remotePort: kind === 'service' ? 7860 : null,
    localPort: kind === 'service' ? 17860 : null,
    healthPath: kind === 'service' ? '/' : null,
    saveDirectories: ['/workspace/my-app/output'],
    storageDirectory: '/content/drive/MyDrive/CloudDeck/my-app',
    autoStart: false,
  };
}
