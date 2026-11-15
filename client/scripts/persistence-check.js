// Explicit live QA only. Reuses the existing VM and never releases it.
(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  const checks = [];
  const waitFor = async (read, timeout = 600000) => {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      const result = await read();
      if (result) return result;
      await pause(1000);
    }
    throw Error('Persistence verification timed out; VM retained');
  };
  const button = text => Array.from(document.querySelectorAll('button')).find(item => item.textContent.trim() === text && !item.disabled);
  const saved = async () => waitFor(async () => {
    const state = await invoke('refresh_cloud');
    if (state.runtime.synchronization === 'failed') throw Error(state.runtime.storage.error || 'Drive verification failed');
    return state.runtime.synchronization === 'verified' ? state : null;
  });
  try {
    await waitFor(() => document.querySelector('nav'), 15000);
    let state;
    for (let attempt = 0; attempt < 3; attempt++) {
      try { state = await invoke('refresh_account'); }
      catch (error) { if (attempt === 2) throw error; }
      if (state?.observation.status === 'current') break;
      if (attempt < 2) await pause(2000 * (attempt + 1));
    }
    if (state?.observation.status !== 'current') throw Error(state?.observation.error || 'Account query failed; VM state is unknown');
    if (state.observation.instances.length !== 1) throw Error('This check requires one existing VM; no VM was allocated');
    state = await invoke('connect_instance', { endpoint: state.observation.instances[0].id });
    if (!['running', 'preparing', 'stopped'].includes(state.runtime.applications.comfyui)) throw Error('An existing managed ComfyUI is required');
    const linuxChecks = await invoke('live_persistence_probe', { action: 'linux_tests' });
    if (!linuxChecks.passed) throw Error('Linux supervisor checks failed: ' + linuxChecks.output);
    checks.push('Linux checks passed, including stopping descendants that ignore SIGTERM');
    const fixture = await invoke('live_persistence_probe', { action: 'prepare' });
    checks.push('Created isolated workflow, settings and input fixtures beside the actual generated image');
    document.querySelector('nav button[aria-label="文件与同步"]').click();
    await waitFor(() => button('保存并校验'), 10000);
    button('保存并校验').click();
    await waitFor(() => document.querySelector('[role="dialog"]'), 10000);
    const confirm = Array.from(document.querySelectorAll('[role="dialog"] button')).find(item => item.textContent.trim() === '保存并校验');
    if (!confirm) throw Error('Save confirmation is missing');
    confirm.click();
    state = await saved();
    if (state.runtime.storage.method !== 'drive_api_md5') throw Error('Verification did not use Drive server metadata');
    if (!Object.values(state.runtime.applications).every(value => value === 'stopped')) throw Error('Applications did not stop before save');
    const output = state.runtime.storage.files.find(item => item.path === fixture.output.path);
    if (output?.sha256 !== fixture.output.sha256) throw Error('Generated output hash did not match');
    checks.push('Native save control stopped applications and verified every configured file against Drive server sizes and checksums');
    const firstGuard = await invoke('live_persistence_probe', { action: 'ready' });
    if (!firstGuard.allowed) throw Error('Valid saved files did not satisfy release guard');
    await invoke('live_persistence_probe', { action: 'mutate' });
    const blocked = await invoke('live_persistence_probe', { action: 'reject_changed' });
    if (!blocked.changedFilesBlockedRelease) throw Error('Changed files did not block release');
    checks.push('Changing a verified workflow blocked release without unassigning the VM');
    await invoke('synchronize_files');
    await saved();
    await invoke('live_persistence_probe', { action: 'recover' });
    await invoke('start_application', { id: 'comfyui' });
    await waitFor(async () => {
      const next = await invoke('refresh_cloud');
      if (next.runtime.applications.comfyui === 'failed') throw Error('ComfyUI restart failed');
      return next.runtime.applicationUrls.comfyui;
    });
    const restored = await invoke('live_persistence_probe', { action: 'inspect' });
    checks.push('Managed restart restored missing workflow, settings and input from Drive with identical SHA-256; original output retained');
    await invoke('synchronize_files');
    state = await saved();
    const releaseGate = await invoke('live_persistence_probe', { action: 'ready' });
    await invoke('refresh_account');
    document.querySelector('nav button[aria-label="文件与同步"]').click();
    await waitFor(() => document.querySelector('.cd-page-content')?.textContent.includes('Drive 服务端已校验'), 10000);
    await pause(500);
    await invoke('native_check_report', { report: { passed: true, kind: 'live-persistence', checks, linuxChecks, fixture, restored, storage: state.runtime.storage, releaseGate, endpoint: state.runtime.endpoint, instanceRetained: true, synchronizationVerified: true, releaseExecuted: false } });
  } catch (error) {
    await invoke('refresh_account').catch(() => {});
    await invoke('native_check_report', { report: { passed: false, kind: 'live-persistence', checks, error: String(error), instanceRetained: true, releaseExecuted: false } });
  }
})();
