export type Readiness = {
  timeoutSeconds?: number;
  intervalSeconds?: number;
  probeTimeoutSeconds?: number;
} & (
  | { type: 'http'; url: string; status?: number }
  | { type: 'tcp'; host: string; port: number }
  | { type: 'command'; command: string; workingDirectory?: string }
);

export function resolveReadiness(
  runtime: { readiness?: Readiness | null; remotePort: number | null; healthPath?: string | null },
  kind: string,
): Readiness | null {
  if (
    !runtime.readiness &&
    kind === 'service' &&
    runtime.healthPath != null &&
    (!runtime.healthPath.startsWith('/') ||
      runtime.healthPath.startsWith('//') ||
      runtime.healthPath.length > 512 ||
      /[\0\r\n]/.test(runtime.healthPath))
  )
    throw new Error('健康检查路径必须以 / 开头。');
  const check =
    runtime.readiness ??
    (kind === 'service' && runtime.remotePort && runtime.healthPath
      ? {
          type: 'http' as const,
          url: `http://127.0.0.1:${runtime.remotePort}${runtime.healthPath}`,
        }
      : null);
  if (kind === 'task') {
    if (check) throw new Error('一次性任务使用退出码判断完成，不能配置 readiness。');
    return null;
  }
  if (!check || !['http', 'tcp', 'command'].includes(check.type))
    throw new Error('服务需要 http、tcp 或 command 类型的 readiness。');
  for (const [field, maximum] of [
    ['timeoutSeconds', 86400],
    ['intervalSeconds', 300],
    ['probeTimeoutSeconds', 30],
  ] as const) {
    const value = check[field];
    if (value !== undefined && (!Number.isInteger(value) || value < 1 || value > maximum))
      throw new Error(`readiness.${field} 必须为 1–${maximum} 的整数。`);
  }
  if (check.type === 'http') {
    const address = new URL(check.url);
    if (
      !['http:', 'https:'].includes(address.protocol) ||
      !address.hostname ||
      address.username ||
      address.password ||
      /[\0\r\n]/.test(check.url)
    )
      throw new Error('就绪检查需要不含账号密码的 HTTP(S) URL。');
    if (
      check.status !== undefined &&
      (!Number.isInteger(check.status) || check.status < 100 || check.status > 599)
    )
      throw new Error('HTTP 预期状态码必须为 100–599。');
  } else if (check.type === 'tcp') {
    if (
      typeof check.host !== 'string' ||
      !check.host ||
      check.host.length > 253 ||
      /[\s/\0]/.test(check.host) ||
      !Number.isInteger(check.port) ||
      check.port < 1 ||
      check.port > 65535
    )
      throw new Error('TCP 就绪检查需要有效的 host 和 port。');
  } else {
    if (
      typeof check.command !== 'string' ||
      !check.command.trim() ||
      new TextEncoder().encode(check.command).length > 4000 ||
      check.command.includes('\0')
    )
      throw new Error('命令就绪检查需要有效的 command。');
    const directory = check.workingDirectory;
    if (
      directory !== undefined &&
      (typeof directory !== 'string' ||
        !directory.startsWith('/') ||
        directory.length > 512 ||
        /[\0\r\n]/.test(directory) ||
        directory.split('/').includes('..'))
    )
      throw new Error('就绪检查工作目录必须为云端绝对路径。');
  }
  return check;
}

export function readinessHttpPath(
  check: Readiness | null,
  remotePort: number | null,
): string | null {
  if (check?.type !== 'http') return null;
  const address = new URL(check.url);
  return address.protocol === 'http:' &&
    ['127.0.0.1', 'localhost'].includes(address.hostname) &&
    Number(address.port || 80) === remotePort
    ? address.pathname + address.search
    : null;
}

export function readinessDescription(check?: Readiness | null): string {
  if (!check) return '按进程退出码判断完成';
  if (check.type === 'http') return `HTTP ${check.url} → ${check.status ?? 200}`;
  if (check.type === 'tcp') return `TCP ${check.host}:${check.port}`;
  return check.command;
}
