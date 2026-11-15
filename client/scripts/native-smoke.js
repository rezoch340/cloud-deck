// Enabled only in the native-smoke build, with isolated local data and no auth.
(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const checks = [];
  const waitFor = async predicate => {
    for (let attempt = 0; attempt < 200; attempt++) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 30));
    }
    throw Error('Native view did not reach the expected state');
  };
  const tick = () => new Promise(resolve => setTimeout(resolve, 70));
  const button = text => Array.from(document.querySelectorAll('button')).find(element => element.textContent.trim() === text);
  try {
    await waitFor(() => document.querySelector('nav button[aria-label="应用"]'));
    const initial = await invoke('get_workspace');
    if (initial.mode !== 'desktop') throw Error('Native IPC is not connected');
    checks.push('Tauri native IPC connected');
    document.querySelector('nav button[aria-label="应用"]').click();
    await waitFor(() => button('添加任务'));
    button('添加任务').click();
    await waitFor(() => document.querySelector('[role="dialog"] [name="name"]'));
    const name = '原生窗口验证';
    const input = document.querySelector('[role="dialog"] [name="name"]');
    input.focus();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, name);
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await tick();
    input.blur();
    await tick();
    document.querySelector('[role="dialog"] button[type="submit"]').click();
    await waitFor(() => !document.querySelector('[role="dialog"]'));
    const saved = await invoke('get_workspace');
    const created = saved.apps.find(app => app.name === name && !app.builtin);
    if (!created || created.kind !== 'task') throw Error('Native configuration save failed');
    checks.push('HeroUI task editor saved through native command');
    const card = Array.from(document.querySelectorAll('.cd-application-card')).find(item => item.textContent.includes(name));
    card.querySelector('button').click();
    await waitFor(() => button('移除本地配置'));
    await waitFor(() => document.querySelector('.cd-application-logs pre'));
    if (!document.querySelector('.cd-application-logs').textContent.includes('部署与运行日志')) throw Error('Generic application log panel is missing');
    checks.push('Every custom task has the shared deployment and runtime log panel');
    button('移除本地配置').click();
    await waitFor(() => button('移除配置'));
    button('移除配置').click();
    await waitFor(() => !document.querySelector('[role="dialog"]'));
    const cleaned = await invoke('get_workspace');
    if (cleaned.apps.some(app => app.id === created.id)) throw Error('Native configuration removal failed');
    checks.push('Native configuration removal confirmed');
    button('添加应用清单').click();
    await waitFor(() => document.querySelector('[name="manifest"]'));
    const manifestInput = document.querySelector('[name="manifest"]');
    const manifest = JSON.parse(manifestInput.value);
    manifest.id = 'native-manifest-check';
    manifest.name = '清单原生验证';
    manifest.kind = 'task';
    manifest.details = '## 自定义应用描述\n\n| 项目 | 内容 |\n| --- | --- |\n| 来源 | 来自清单的只读表格 |\n\n<script>window.descriptionExecuted = true</script>';
    Object.assign(manifest.runtime, { command: 'echo native-manifest', remotePort: null, localPort: null, healthPath: null, readiness: null });
    const setManifestText = value => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(document.querySelector('[name="manifest"]'), value);
      document.querySelector('[name="manifest"]').dispatchEvent(new Event('input', { bubbles: true }));
    };
    setManifestText(JSON.stringify(manifest, null, 2));
    await tick();
    document.querySelector('[role="dialog"] button[type="submit"]').click();
    await waitFor(() => !document.querySelector('[role="dialog"]'));
    const configured = await invoke('get_workspace');
    if (!configured.apps.some(application => application.id === manifest.id && application.manifest?.runtime.command === 'echo native-manifest')) throw Error('Manifest editor did not persist canonical JSON');
    checks.push('Canonical lifecycle manifest saved through the HeroUI JSON editor');
    await waitFor(() => document.querySelector('.cd-markdown table')?.textContent.includes('来自清单的只读表格'));
    if (window.descriptionExecuted || document.querySelector('.cd-markdown script') || document.querySelector('.cd-markdown input:not([disabled])')) throw Error('Application description was not read-only');
    checks.push('Custom application Markdown description rendered a read-only table without executable HTML');
    button('编辑配置').click();
    await waitFor(() => document.querySelector('[name="manifest"]'));
    setManifestText('{');
    await tick();
    document.querySelector('[role="dialog"] button[type="submit"]').click();
    await waitFor(() => document.querySelector('[role="dialog"] [role="alert"]'));
    button('取消').click();
    await waitFor(() => !document.querySelector('[role="dialog"]'));
    const retained = await invoke('get_workspace');
    if (!retained.apps.some(application => application.id === manifest.id && application.manifest?.runtime.command === 'echo native-manifest')) throw Error('Invalid JSON replaced the saved manifest');
    checks.push('Invalid manifest JSON retained an inline error and preserved the saved configuration');
    button('移除本地配置').click();
    await waitFor(() => button('移除配置'));
    button('移除配置').click();
    await waitFor(() => !document.querySelector('[role="dialog"]'));
    const builtin = Array.from(document.querySelectorAll('.cd-application-card')).find(item => item.textContent.includes('ComfyUI'));
    builtin.querySelector('button').click();
    await waitFor(() => document.querySelector('.cd-page-content').textContent.includes('setup.py'));
    checks.push('Bundled ComfyUI configuration rendered in native WebView');
    button('查看 manifest.json').click();
    await waitFor(() => document.querySelector('[name="manifest"]'));
    const canonical = JSON.parse(document.querySelector('[name="manifest"]').value);
    if (canonical.resources.files.length !== 3 || 'installation' in canonical || 'usePreparedLaunch' in canonical.runtime || 'documents' in canonical.resources || canonical.shutdown.save.length !== 3) throw Error('Bundled lifecycle manifest did not contain the expected resources and save rules');
    button('关闭').click();
    await waitFor(() => !document.querySelector('[role="dialog"]'));
    checks.push('Built-in manifest viewer contains three resources and three save rules with ordinary commands and no installation schema');
    if (initial.observation.lastInstance?.regionLabel) {
      document.querySelector('nav button[aria-label="云端工作台"]').click();
      await waitFor(() => document.querySelector('.cd-machine-identity')?.textContent.includes(initial.observation.lastInstance.regionLabel));
      if (!document.querySelector('.cd-machine-identity').textContent.includes('上次分配地区')) throw Error('Released instance region was not marked as history');
      checks.push('Saved allocated region rendered with an explicit historical label in native WebView');
    }
    await tick();
    await invoke('native_check_report', { report: { passed: true, checks, viewport: [innerWidth, innerHeight] } });
  } catch (error) {
    await invoke('native_check_report', { report: { passed: false, checks, error: String(error) } });
  }
})();
