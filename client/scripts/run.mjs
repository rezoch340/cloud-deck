import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const project = fileURLToPath(new URL('..', import.meta.url));
const workRoot = process.env.CLOUD_DECK_WORK_ROOT || (process.platform === 'darwin' ? '/Volumes/CODEX_SCRATCH' : '');
if (!workRoot || !fs.existsSync(workRoot) || !path.isAbsolute(workRoot)) {
  throw new Error('请先设置可用的外置工作目录 CLOUD_DECK_WORK_ROOT；构建不会回落到系统盘。');
}
if (process.platform === 'darwin' && workRoot.startsWith('/Volumes/CODEX_SCRATCH') && !fs.existsSync('/Volumes/CODEX_SCRATCH/.codex-scratch-volume')) {
  throw new Error('CODEX_SCRATCH 未挂载，请先挂载外置 scratch 卷；不会使用系统临时目录。');
}
const build = path.join(workRoot, 'build/cloud-deck/client');
const temporary = path.join(workRoot, 'tmp/cloud-deck/client');
const cache = path.join(workRoot, 'cache');
const dirs = [build, temporary, 'cargo', 'npm', 'pnpm', 'clang', 'tauri', 'xdg', 'rustup'];
for (const dir of dirs) fs.mkdirSync(path.isAbsolute(dir) ? dir : path.join(cache, dir), { recursive: true });

function linkDirectory(link, target) {
  fs.mkdirSync(target, { recursive: true });
  const existing = fs.lstatSync(link, { throwIfNoEntry: false });
  if (existing) {
    if (fs.existsSync(link) && fs.realpathSync(link) === fs.realpathSync(target)) return;
    if (!existing.isSymbolicLink()) throw new Error(`外置构建入口不能覆盖真实目录：${link}`);
    fs.unlinkSync(link); // Only repoint the link; retain both work roots and their contents.
  }
  fs.symlinkSync(target, link, process.platform === 'win32' ? 'junction' : 'dir');
}
linkDirectory(path.join(project, 'src-tauri/gen'), path.join(build, 'gen'));
linkDirectory(path.join(project, 'src-tauri/icons'), path.join(build, 'icons'));
linkDirectory(path.join(project, 'node_modules'), path.join(build, 'node_modules'));
// 使用固定相对路径供 Tauri 嵌入界面；实际产物仍在外盘，Windows 使用可跨盘的 junction。
linkDirectory(path.join(project, 'dist'), path.join(build, 'dist'));
const host = process.platform === 'darwin' ? `${process.arch === 'arm64' ? 'aarch64' : 'x86_64'}-apple-darwin` : `${process.arch === 'arm64' ? 'aarch64' : 'x86_64'}-pc-windows-msvc`;
const toolchainHome = process.env.RUSTUP_HOME || path.join(os.homedir(), '.rustup');
const defaultRustBin = path.join(toolchainHome, 'toolchains', `stable-${host}`, 'bin');
let detectedCargoDir = '';
if (!process.env.CLOUD_DECK_RUST_BIN && !fs.existsSync(defaultRustBin)) {
  const lookup = spawnSync(process.platform === 'win32' ? 'where.exe' : 'which', ['cargo'], { encoding: 'utf8' });
  const firstLine = (lookup.stdout || '').trim().split(/\r?\n/)[0];
  if (firstLine && fs.existsSync(firstLine)) detectedCargoDir = path.dirname(firstLine);
}
const rustBin = process.env.CLOUD_DECK_RUST_BIN || detectedCargoDir || defaultRustBin;
const exe = process.platform === 'win32' ? '.exe' : '';
const env = {
  ...process.env,
  CLOUD_DECK_BUILD_ROOT: build,
  CLOUD_DECK_DATA_DIR: process.env.CLOUD_DECK_DATA_DIR || path.join(workRoot, 'data/cloud-deck/client'),
  // 本机默认将 Cargo / Rustup 缓存固定在外置卷；CI 显式传入环境变量时保留既有工具链目录以便跨架构编译。
  CARGO_HOME: process.env.CARGO_HOME || path.join(cache, 'cargo'), CARGO_TARGET_DIR: path.join(build, 'target'),
  RUSTUP_HOME: process.env.RUSTUP_HOME || path.join(cache, 'rustup'), RUSTC: path.join(rustBin, `rustc${exe}`), RUSTDOC: path.join(rustBin, `rustdoc${exe}`),
  NPM_CONFIG_CACHE: path.join(cache, 'npm'), PNPM_HOME: path.join(cache, 'pnpm'),
  XDG_CACHE_HOME: path.join(cache, 'xdg'), XDG_DATA_HOME: path.join(workRoot, 'data/xdg'), XDG_STATE_HOME: path.join(workRoot, 'data/xdg-state'),
  CLANG_MODULE_CACHE_PATH: path.join(cache, 'clang'), TAURI_CACHE_DIR: path.join(cache, 'tauri'),
  TMPDIR: temporary, TMP: temporary, TEMP: temporary,
  // Cargo's bundled objcopy needs the existing toolchain's LLVM library on macOS.
  ...(process.platform === 'darwin' ? {
    DYLD_FALLBACK_LIBRARY_PATH: [path.resolve(rustBin, '../lib'), process.env.DYLD_FALLBACK_LIBRARY_PATH].filter(Boolean).join(path.delimiter),
  } : {}),
  PATH: `${rustBin}${path.delimiter}${process.env.PATH || ''}`,
};
const mode = process.argv[2];
const extra = process.argv.slice(3);
const moduleFile = (...parts) => path.join(project, 'node_modules', ...parts);
function run(command, args) {
  const result = spawnSync(command, args, { cwd: project, env, stdio: 'inherit', shell: false });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status || 1);
}
function webBuild() {
  run(process.execPath, [moduleFile('typescript/bin/tsc'), '--noEmit']);
  run(process.execPath, [moduleFile('vite/bin/vite.js'), 'build']);
}
function desktopBuild(args) {
  webBuild();
  const localConfig = path.join(temporary, 'tauri-build.json');
  // 带盘符的绝对路径会被 Tauri 当成 URL，导致发行包没有嵌入前端文件。
  const configuration = { build: { frontendDist: '../dist' } };
  if (process.platform === 'darwin') {
    let identity = process.env.CLOUD_DECK_SIGNING_IDENTITY;
    if (!identity) {
      const result = spawnSync('/usr/bin/security', ['find-identity', '-v', '-p', 'codesigning'], { env, encoding: 'utf8' });
      const identities = [...(result.stdout || '').matchAll(/\b([A-F0-9]{40}) "(?:Apple Development|Developer ID Application):[^"]+"/g)];
      if (identities.length === 1) identity = identities[0][1];
    }
    if (identity) {
      configuration.bundle = { macOS: { signingIdentity: identity } };
      console.log(identity === '-' ? 'Using ad-hoc signing.' : 'Using the existing macOS signing identity across builds.');
    } else {
      console.log('No unique macOS signing identity selected; ad-hoc builds may request Keychain access again.');
    }
  }
  fs.writeFileSync(localConfig, JSON.stringify(configuration));
  run(process.execPath, [moduleFile('@tauri-apps/cli/tauri.js'), 'build', '--config', localConfig, ...args]);
}
function packageManager() {
  if (process.env.npm_execpath && /pnpm/i.test(path.basename(process.env.npm_execpath))) return process.env.npm_execpath;
  for (const directory of (process.env.PATH || '').split(path.delimiter)) {
    for (const name of process.platform === 'win32' ? ['pnpm.exe', 'pnpm.cmd'] : ['pnpm']) {
      const candidate = path.join(directory, name);
      if (!fs.existsSync(candidate)) continue;
      if (name.endsWith('.cmd')) {
        const script = path.join(directory, 'node_modules/pnpm/bin/pnpm.cjs');
        if (fs.existsSync(script)) return script;
        const corepackScript = path.join(directory, 'node_modules/corepack/dist/pnpm.js');
        if (fs.existsSync(corepackScript)) return corepackScript;
        return candidate;
      } else return fs.realpathSync(candidate);
    }
  }
  throw new Error('未找到现有 pnpm；构建不会自动向系统盘安装包管理器。');
}
const manifest = path.join(project, 'src-tauri/Cargo.toml');
if (['rust-check', 'rust-test', 'desktop-build', 'native-check', 'live-check'].includes(mode) && !fs.existsSync(path.join(build, 'icons/icon.png'))) {
  run(process.execPath, [moduleFile('@tauri-apps/cli/tauri.js'), 'icon', path.join(project, 'app-icon.svg'), '--output', path.join(build, 'icons')]);
}
if (mode === 'install') {
  const manager = packageManager();
  const args = ['install', '--store-dir', path.join(cache, 'pnpm/store'), '--modules-dir', path.join(build, 'node_modules'), '--ignore-scripts', extra.includes('--update-lockfile') ? '--no-frozen-lockfile' : '--frozen-lockfile'];
  if (/\.(cjs|mjs|js)$/.test(manager)) run(process.execPath, [manager, ...args]);
  else if (process.platform === 'win32' && /\.cmd$/i.test(manager)) run(process.env.ComSpec || 'cmd.exe', ['/d', '/s', '/c', manager, ...args]);
  else run(manager, args);
} else if (mode === 'dev') run(process.execPath, [moduleFile('vite/bin/vite.js'), '--host', '127.0.0.1', '--port', '18774', '--strictPort']);
else if (mode === 'web-build') webBuild();
else if (mode === 'test') run(process.execPath, ['--test', 'src/domain.test.ts', 'src/manifest.test.ts']);
else if (mode === 'agent-test') run('python3', ['-B', '-m', 'unittest', 'discover', '-s', '..', '-p', 'test_cloud*.py', '-v', ...extra]);
else if (mode === 'format' || mode === 'format-check') run(process.execPath, [moduleFile('prettier/bin/prettier.cjs'), mode === 'format' ? '--write' : '--check', 'src/**/*.{ts,tsx,css}', 'vite.config.ts']);
else if (mode === 'rust-test') run(path.join(rustBin, `cargo${exe}`), ['test', '--manifest-path', manifest, '--no-default-features', ...extra]);
else if (mode === 'rust-check') run(path.join(rustBin, `cargo${exe}`), ['check', '--manifest-path', manifest, '--no-default-features', ...extra]);
else if (mode === 'rust-fmt') run(path.join(rustBin, `cargo${exe}`), ['fmt', '--manifest-path', manifest, ...extra]);
else if (mode === 'desktop-build') desktopBuild(extra);
else if (mode === 'native-check' || mode === 'live-check') {
  const live = mode === 'live-check';
  const manifestCheck = live && extra.includes('--manifest');
  const comfy = live && (extra.includes('--comfy') || manifestCheck);
  const persistence = live && extra.includes('--persistence');
  const shutdown = live && extra.includes('--shutdown');
  const keepOpen = (comfy || persistence || shutdown) && extra.includes('--keep-open');
  if (process.platform !== 'darwin') throw new Error('原生窗口截图验证当前只在 macOS 实现；Windows 未实测。');
  if (!extra.includes('--reuse-build')) desktopBuild(['--debug', '--features', 'native-smoke', '--bundles', 'app']);
  const product = JSON.parse(fs.readFileSync(path.join(project, 'src-tauri/tauri.conf.json'), 'utf8')).productName;
  const binDirectory = path.join(build, 'target/debug/bundle/macos', `${product}.app`, 'Contents/MacOS');
  const binaries = fs.readdirSync(binDirectory).filter(name => fs.statSync(path.join(binDirectory, name)).isFile());
  if (binaries.length !== 1) throw new Error('无法唯一识别测试应用的主程序。');
  const directory = fs.mkdtempSync(path.join(temporary, live ? 'live-check-' : 'native-smoke-'));
  const fixtureOption = extra.indexOf('--region-fixture');
  if (!live && fixtureOption !== -1) {
    const fixture = JSON.parse(fs.readFileSync(extra[fixtureOption + 1], 'utf8'));
    const lastInstance = Object.fromEntries(['id', 'accelerator', 'region', 'regionLabel'].map(key => [key, fixture[key] ?? null]));
    fs.writeFileSync(path.join(directory, 'workspace.json'), JSON.stringify({ schema: 1, customApps: [],
      account: { subject: 'offline-region-check', name: '离线地区显示验证', email: 'region-check@example.invalid', status: 'saved' },
      observation: { status: 'cached', observedAt: Date.parse(fixture.recordedAt), balance: null, hourlyRate: null, eligibleGpus: [], instances: [], lastInstance, error: null },
      events: [{ time: Date.now(), level: 'info', category: 'instance', message: '离线窗口验证：显示已保存的真实地区记录，不访问 Google。' }],
    }));
  }
  const log = fs.openSync(path.join(directory, 'native-process.log'), 'w');
  const reportPath = path.join(directory, 'native-check.json');
  const options = { cwd: workRoot, env: { ...env, CLOUD_DECK_SMOKE: live ? '0' : '1', CLOUD_DECK_LIVE_CHECK: live ? '1' : '0', CLOUD_DECK_COMFY_CHECK: comfy ? '1' : '0', CLOUD_DECK_MANIFEST_CHECK: manifestCheck ? '1' : '0', CLOUD_DECK_PERSISTENCE_CHECK: persistence ? '1' : '0', CLOUD_DECK_SHUTDOWN_CHECK: shutdown ? '1' : '0', CLOUD_DECK_KEEP_OPEN: keepOpen ? '1' : '0', CLOUD_DECK_DATA_DIR: directory }, stdio: ['ignore', log, log] };
  const timeout = comfy ? 3300000 : live ? 1200000 : 45000;
  let result;
  try {
    if (!keepOpen) result = spawnSync(path.join(binDirectory, binaries[0]), [], { ...options, timeout });
    else {
      const child = spawn(path.join(binDirectory, binaries[0]), [], { ...options, detached: true });
      result = await new Promise(resolve => {
        const deadline = Date.now() + timeout;
        let finished = false;
        const finish = value => { if (finished) return; finished = true; clearInterval(timer); resolve(value); };
        const timer = setInterval(() => {
          if (fs.existsSync(reportPath)) {
            try {
              const report = JSON.parse(fs.readFileSync(reportPath, 'utf8'));
              if (report.passed && report.nativeProcessRetained && child.exitCode === null) {
                child.unref(); finish({ status: 0, nativeProcessId: child.pid }); return;
              }
              if (!report.passed) { finish({ status: 1 }); return; }
            } catch { /* WebView may still be writing the report. */ }
          }
          if (Date.now() > deadline) { child.kill(); finish({ status: 1, error: 'Native check timed out' }); }
        }, 250);
        child.once('error', error => finish({ status: 1, error }));
        child.once('exit', status => finish({ status: status ?? 1 }));
      });
    }
  } finally { fs.closeSync(log); }
  if (!fs.existsSync(reportPath)) throw new Error(`原生窗口验证未完成，日志位于 ${directory}`);
  const report = JSON.parse(fs.readFileSync(reportPath, 'utf8'));
  console.log(JSON.stringify({ directory, ...report, ...(result.nativeProcessId ? { nativeProcessId: result.nativeProcessId } : {}) }, null, 2));
  if (result.error || result.status !== 0 || !report.passed || !report.nativeScreenshot || (keepOpen && !result.nativeProcessId)) process.exit(1);
} else if (mode === 'paths') {
  console.log(JSON.stringify(Object.fromEntries(Object.entries(env).filter(([key]) => /^(CLOUD_DECK_|CARGO_|RUSTUP_HOME|CLANG_|TAURI_|TMP|TEMP|XDG_|NPM_CONFIG_CACHE|PNPM_HOME)/.test(key))), null, 2));
} else throw new Error('Expected install, dev, web-build, test, format, format-check, rust-test, rust-check, rust-fmt, desktop-build, native-check, live-check or paths.');
