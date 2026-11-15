// 版本同步脚本的 TypeScript 类型声明，供客户端测试与构建检查使用。
export function normalizeReleaseVersion(rawValue: string): string;
export function syncClientVersion(
  clientRoot: string,
  rawVersion: string,
  checkOnly?: boolean,
): string;
