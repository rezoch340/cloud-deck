import type { Recipe } from './domain.ts';
import { validateRecipe } from './domain.ts';
import { resolveReadiness, readinessHttpPath, type Readiness } from './readiness.ts';

export interface FileRule {
  source: string;
  target: string;
  optional?: boolean;
}

export type ResourceSource = { kind: 'drive'; path: string } | { kind: 'download'; url: string };

export interface ApplicationManifest {
  schemaVersion: 1;
  id: string;
  name: string;
  description: string;
  details?: string;
  category?: 'audio' | 'video' | 'llm' | 'workflow' | 'general';
  kind: 'service' | 'task';
  runtime: {
    command: string;
    workingDirectory: string;
    environment: Record<string, string>;
    remotePort: number | null;
    localPort: number | null;
    healthPath?: string | null;
    readiness?: Readiness | null;
  };
  resources: {
    files: { target: string; size: number; sha256: string; sources: ResourceSource[] }[];
  };
  startup: {
    restore: FileRule[];
    commands: { command: string; workingDirectory: string }[];
  };
  shutdown: { save: FileRule[] };
}

export function parseApplicationManifest(text: string): ApplicationManifest {
  if (new TextEncoder().encode(text).length > 512 * 1024) {
    throw new Error('应用清单不能超过 512 KiB。');
  }
  const manifest = JSON.parse(text) as ApplicationManifest;
  if (
    manifest?.category !== undefined &&
    !['audio', 'video', 'llm', 'workflow', 'general'].includes(manifest.category)
  ) {
    throw new Error('应用分类必须为 audio、video、llm、workflow 或 general。');
  }
  if (
    manifest?.schemaVersion !== 1 ||
    typeof manifest.id !== 'string' ||
    !/^[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}$/.test(manifest.id) ||
    typeof manifest.name !== 'string' ||
    !['service', 'task'].includes(manifest.kind) ||
    typeof manifest.runtime?.command !== 'string' ||
    typeof manifest.runtime?.workingDirectory !== 'string' ||
    !Array.isArray(manifest.startup?.commands) ||
    !Array.isArray(manifest.startup?.restore) ||
    !Array.isArray(manifest.shutdown?.save) ||
    !Array.isArray(manifest.resources?.files)
  ) {
    throw new Error('清单需要 schemaVersion、应用信息、runtime、resources、startup 和 shutdown。');
  }
  const absolute = (value: unknown): value is string =>
    typeof value === 'string' &&
    value.startsWith('/') &&
    value !== '/' &&
    value.length <= 512 &&
    !/[\0\n\r]/.test(value) &&
    !value.split('/').includes('..');
  if (
    manifest.details !== undefined &&
    (typeof manifest.details !== 'string' ||
      new TextEncoder().encode(manifest.details).length > 64 * 1024)
  ) {
    throw new Error('details 必须为不超过 64 KiB 的 Markdown 描述。');
  }
  const local = (value: unknown): value is string =>
    absolute(value) && value !== '/content/drive' && !value.startsWith('/content/drive/');
  const drive = (value: unknown): value is string =>
    absolute(value) &&
    value.startsWith('/content/drive/MyDrive/') &&
    value.length > '/content/drive/MyDrive/'.length;
  if (
    !manifest.runtime.environment ||
    typeof manifest.runtime.environment !== 'object' ||
    Array.isArray(manifest.runtime.environment)
  ) {
    throw new Error('runtime.environment 必须为对象。');
  }
  if (
    [
      manifest.startup.restore,
      manifest.startup.commands,
      manifest.shutdown.save,
      manifest.resources.files,
    ].some((entries) => entries.length > 128)
  )
    throw new Error('单个清单列表不能超过 128 项。');
  for (const rule of manifest.startup.restore) {
    if (
      !drive(rule?.source) ||
      !local(rule?.target) ||
      (rule.optional !== undefined && typeof rule.optional !== 'boolean')
    )
      throw new Error('启动恢复需要 Drive 来源、实例目标和布尔 optional。');
  }
  for (const rule of manifest.shutdown.save) {
    if (!local(rule?.source) || !drive(rule?.target))
      throw new Error('关机保存需要实例来源和 Drive 目标。');
  }
  for (const rule of [...manifest.startup.restore, ...manifest.shutdown.save]) {
    if (!absolute(rule?.source) || !absolute(rule?.target)) {
      throw new Error('同步规则的 source、target 必须是绝对路径。');
    }
  }
  for (const step of manifest.startup.commands) {
    if (!step?.command?.trim() || !step.workingDirectory?.startsWith('/')) {
      throw new Error('准备命令需要 command 和 workingDirectory。');
    }
  }
  for (const resource of manifest.resources.files) {
    if (
      !local(resource?.target) ||
      !Number.isSafeInteger(resource.size) ||
      resource.size <= 0 ||
      !/^[a-fA-F0-9]{64}$/.test(resource.sha256) ||
      !Array.isArray(resource.sources) ||
      !resource.sources.length ||
      resource.sources.length > 128
    ) {
      throw new Error('资源需要目标、大小、SHA-256 和按优先级排列的 sources。');
    }
    for (const source of resource.sources) {
      if (source?.kind === 'drive' && drive(source.path)) continue;
      if (source?.kind === 'download' && typeof source.url === 'string') {
        const address = new URL(source.url);
        if (
          address.protocol === 'https:' &&
          address.hostname &&
          !address.username &&
          !address.password
        )
          continue;
      }
      throw new Error('资源来源必须是 Drive 路径或 HTTPS 下载地址。');
    }
  }
  if ('installation' in manifest || 'usePreparedLaunch' in manifest.runtime)
    throw new Error('请使用 command 执行安装和启动命令或脚本。');
  const runtimeError = validateRecipe(
    { ...manifestRecipe(manifest), id: null, prepareCommand: null },
    [],
  );
  if (runtimeError) throw new Error(runtimeError);
  return manifest;
}

export function manifestRecipe(manifest: ApplicationManifest, builtin = false): Recipe {
  const readiness = resolveReadiness(manifest.runtime, manifest.kind);
  return {
    id: manifest.id,
    name: manifest.name,
    description: manifest.description || '',
    kind: manifest.kind,
    ...manifest.runtime,
    readiness,
    healthPath: readinessHttpPath(readiness, manifest.runtime.remotePort),
    prepareCommand: manifest.startup.commands.map((step) => step.command).join('\n') || null,
    saveDirectories: manifest.shutdown.save.map((rule) => rule.source),
    storageDirectory:
      manifest.shutdown.save[0]?.target.split('/').slice(0, -1).join('/') ||
      `/content/drive/MyDrive/CloudDeck/${manifest.id}`,
    autoStart: false,
    builtin,
    manifest,
  };
}

export function newApplicationManifest(): ApplicationManifest {
  return {
    schemaVersion: 1,
    id: `app-${crypto.randomUUID()}`,
    name: '我的应用',
    description: '',
    kind: 'service',
    runtime: {
      command: 'exec python3 -m http.server 7860 --bind 127.0.0.1',
      workingDirectory: '/opt/my-app',
      environment: {},
      remotePort: 7860,
      localPort: 17860,
      readiness: { type: 'http', url: 'http://127.0.0.1:7860/' },
    },
    resources: { files: [] },
    startup: {
      restore: [],
      commands: [{ command: 'mkdir -p /opt/my-app', workingDirectory: '/' }],
    },
    shutdown: { save: [] },
  };
}
