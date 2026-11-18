import filesystem from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';

// Tauri 先签内部代码；只在新建的 staging 副本上清理属性并封装最终应用。
export function packageMacos({ project, build, temporary, env }) {
  const configuration = JSON.parse(filesystem.readFileSync(path.join(project, 'src-tauri/tauri.conf.json'), 'utf8'));
  const appName = `${configuration.productName}.app`;
  const sourceApp = path.join(build, 'target/release/bundle/macos', appName);
  const staging = filesystem.mkdtempSync(path.join(temporary, 'macos-package-'));
  const stagedApp = path.join(staging, appName);
  const execute = (command, args) => {
    const result = spawnSync(command, args, { env, encoding: 'utf8' });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`${path.basename(command)} 失败：${result.stderr || result.stdout || result.signal || result.status}`);
    return `${result.stdout || ''}${result.stderr || ''}`.trim();
  };
  const plistValue = (application, field) => execute('/usr/libexec/PlistBuddy', ['-c', `Print :${field}`, path.join(application, 'Contents/Info.plist')]);
  const verifyApp = application => {
    execute('/usr/bin/codesign', ['--verify', '--deep', '--strict', '--verbose=2', application]);
    const signature = execute('/usr/bin/codesign', ['--display', '--verbose=4', application]);
    if (!/^Signature=adhoc$/m.test(signature) || !/^CodeDirectory .*flags=.*\bruntime\b/m.test(signature)) {
      throw new Error('分发包必须使用 ad-hoc 签名并启用 hardened runtime。');
    }
    if (plistValue(application, 'CFBundleIdentifier') !== configuration.identifier
      || plistValue(application, 'CFBundleShortVersionString') !== configuration.version) {
      throw new Error('应用标识或版本与当前源码不一致。');
    }
    const executable = path.join(application, 'Contents/MacOS', plistValue(application, 'CFBundleExecutable'));
    const architectures = execute('/usr/bin/lipo', ['-archs', executable]);
    const expected = process.arch === 'arm64' ? 'arm64' : 'x86_64';
    if (architectures !== expected) throw new Error(`预期 ${expected}，实际构建架构为 ${architectures}。`);
    // 此入口不打开窗口、不访问钥匙串、不连接 Google 或 Colab。
    execute(executable, ['--check-frontend-assets']);
    return { architectures, signature };
  };

  try {
    execute('/usr/bin/ditto', ['--norsrc', '--noextattr', '--noacl', sourceApp, stagedApp]);
    execute('/usr/bin/xattr', ['-cr', stagedApp]);
    execute('/usr/bin/codesign', ['--force', '--options', 'runtime', '--timestamp=none', '--sign', '-', stagedApp]);
    verifyApp(stagedApp);

    const archiveName = `CloudDeck-${configuration.version}-macos-${process.arch}.app.zip`;
    const stagedArchive = path.join(staging, archiveName);
    execute('/usr/bin/ditto', ['-c', '-k', '--norsrc', '--noextattr', '--noacl', '--keepParent', stagedApp, stagedArchive]);
    const entries = execute('/usr/bin/unzip', ['-Z1', stagedArchive]).split('\n');
    if (entries.some(entry => entry.startsWith('__MACOSX/') || entry.split('/').some(part => part.startsWith('._')))) {
      throw new Error('ZIP 包含扩展属性 sidecar 文件，停止交付。');
    }
    const extracted = path.join(staging, 'unpacked');
    execute('/usr/bin/ditto', ['-x', '-k', stagedArchive, extracted]);
    const verification = verifyApp(path.join(extracted, appName));
    const checksum = createHash('sha256').update(filesystem.readFileSync(stagedArchive)).digest('hex');

    // 用已验签的解压副本制作 Finder 拖放安装映像。
    const imageName = `CloudDeck-${configuration.version}-macos-${process.arch}.dmg`;
    const imagePath = path.join(staging, imageName);
    filesystem.symlinkSync('/Applications', path.join(extracted, 'Applications'));
    execute('/usr/bin/hdiutil', ['create', '-fs', 'HFS+', '-format', 'UDZO', '-volname', configuration.productName, '-srcfolder', extracted, imagePath]);
    execute('/usr/bin/codesign', ['--force', '--timestamp=none', '--sign', '-', imagePath]);
    execute('/usr/bin/codesign', ['--verify', '--strict', imagePath]);
    execute('/usr/bin/hdiutil', ['verify', imagePath]);
    const imageChecksum = createHash('sha256').update(filesystem.readFileSync(imagePath)).digest('hex');

    const artifacts = path.join(build, 'artifacts');
    filesystem.mkdirSync(artifacts, { recursive: true });
    const output = filesystem.mkdtempSync(path.join(artifacts, `macos-${process.arch}-`));
    filesystem.copyFileSync(stagedArchive, path.join(output, archiveName));
    filesystem.copyFileSync(imagePath, path.join(output, imageName));
    filesystem.writeFileSync(path.join(output, 'SHA256SUMS.txt'), `${checksum}  ${archiveName}\n${imageChecksum}  ${imageName}\n`);
    filesystem.copyFileSync(path.join(project, 'macos-install.md'), path.join(output, 'INSTALL.md'));
    filesystem.writeFileSync(path.join(output, 'verification.json'), `${JSON.stringify({
      version: configuration.version, archive: archiveName, sha256: checksum,
      diskImage: imageName, diskImageSha256: imageChecksum,
      ...verification, notarized: false, archiveSignatureVerified: true, embeddedFrontendVerified: true,
    }, null, 2)}\n`);
    console.log(`macOS DMG 与 ZIP 已生成：${output}\nDMG SHA-256: ${imageChecksum}`);
  } finally {
    filesystem.rmSync(staging, { recursive: true, force: true });
  }
}
