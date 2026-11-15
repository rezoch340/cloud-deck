// 将 Git 标签或命令行指定的语义化版本号同步到前端、Tauri 配置与 Rust 包清单。
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SEMVER_PATTERN =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?$/;

/** 规范化并校验版本号（支持剥离 `refs/tags/` 与前导 `v`）。 */
export function normalizeReleaseVersion(rawValue) {
  if (typeof rawValue !== 'string' || !rawValue.trim()) {
    throw new Error('缺少版本号参数，请提供形如 v0.1.0 或 0.1.0 的语义化版本。');
  }
  const withoutRef = rawValue.trim().replace(/^refs\/tags\//, '');
  const cleaned = withoutRef.replace(/^[vV]/, '');
  if (!SEMVER_PATTERN.test(cleaned)) {
    throw new Error(`版本号格式无效（需符合 x.y.z）：${rawValue}`);
  }
  return cleaned;
}

/** 同步或检查客户端四个版本元数据文件，返回规范化后的版本号。 */
export function syncClientVersion(clientRoot, rawVersion, checkOnly = false) {
  const version = normalizeReleaseVersion(rawVersion);
  const packagePath = path.join(clientRoot, 'package.json');
  const tauriConfPath = path.join(clientRoot, 'src-tauri/tauri.conf.json');
  const cargoTomlPath = path.join(clientRoot, 'src-tauri/Cargo.toml');
  const cargoLockPath = path.join(clientRoot, 'src-tauri/Cargo.lock');

  const packageData = JSON.parse(fs.readFileSync(packagePath, 'utf8'));
  const tauriConfData = JSON.parse(fs.readFileSync(tauriConfPath, 'utf8'));
  const cargoTomlText = fs.readFileSync(cargoTomlPath, 'utf8');
  const cargoLockText = fs.existsSync(cargoLockPath) ? fs.readFileSync(cargoLockPath, 'utf8') : null;

  const cargoTomlMatch = cargoTomlText.match(
    /(\[package\][\s\S]*?\nname\s*=\s*"cloud-deck"\r?\nversion\s*=\s*")([^"]+)(")/,
  );
  if (!cargoTomlMatch) {
    throw new Error('无法在 Cargo.toml 中定位 cloud-deck 的 [package] version 字段。');
  }

  if (checkOnly) {
    const mismatches = [];
    if (packageData.version !== version) mismatches.push(`package.json (${packageData.version})`);
    if (tauriConfData.version !== version) {
      mismatches.push(`tauri.conf.json (${tauriConfData.version})`);
    }
    if (cargoTomlMatch[2] !== version) mismatches.push(`Cargo.toml (${cargoTomlMatch[2]})`);
    if (cargoLockText) {
      const lockMatch = cargoLockText.match(
        /(\[\[package\]\]\r?\nname\s*=\s*"cloud-deck"\r?\nversion\s*=\s*")([^"]+)(")/,
      );
      if (lockMatch && lockMatch[2] !== version) {
        mismatches.push(`Cargo.lock (${lockMatch[2]})`);
      }
    }
    if (mismatches.length > 0) {
      throw new Error(`客户端版本号不一致，期望 ${version}，实际：${mismatches.join(', ')}`);
    }
    return version;
  }

  packageData.version = version;
  fs.writeFileSync(packagePath, `${JSON.stringify(packageData, null, 2)}\n`);

  tauriConfData.version = version;
  fs.writeFileSync(tauriConfPath, `${JSON.stringify(tauriConfData, null, 2)}\n`);

  const nextCargoToml = cargoTomlText.replace(
    /(\[package\][\s\S]*?\nname\s*=\s*"cloud-deck"\r?\nversion\s*=\s*")([^"]+)(")/,
    `$1${version}$3`,
  );
  fs.writeFileSync(cargoTomlPath, nextCargoToml);

  if (cargoLockText) {
    const nextCargoLock = cargoLockText.replace(
      /(\[\[package\]\]\r?\nname\s*=\s*"cloud-deck"\r?\nversion\s*=\s*")([^"]+)(")/,
      `$1${version}$3`,
    );
    fs.writeFileSync(cargoLockPath, nextCargoLock);
  }

  return version;
}

const currentScriptPath = fileURLToPath(import.meta.url);
if (process.argv[1] && path.resolve(process.argv[1]) === currentScriptPath) {
  const cliArgs = process.argv.slice(2);
  const checkOnly = cliArgs.includes('--check');
  const githubOutput = cliArgs.includes('--github-output');
  const versionArg =
    cliArgs.find((argument) => !argument.startsWith('--')) || process.env.GITHUB_REF_NAME || '';
  const clientRoot = path.resolve(path.dirname(currentScriptPath), '..');
  const syncedVersion = syncClientVersion(clientRoot, versionArg, checkOnly);
  if (githubOutput && process.env.GITHUB_OUTPUT) {
    fs.appendFileSync(process.env.GITHUB_OUTPUT, `version=${syncedVersion}\n`);
  }
  console.log(syncedVersion);
}
