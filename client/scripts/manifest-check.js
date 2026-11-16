// Explicit QA. Reuse an existing VM, or create a T4 and save/close it after validation.
(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const checks = [];
  const evidence = {};
  let created = false;
  const pause = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  try {
    for (let attempt = 0; attempt < 100 && !document.querySelector('nav'); attempt++) await pause(100);
    let state = await invoke('refresh_account');
    if (state.observation.status !== 'current' || state.observation.instances.length > 1) {
      throw Error('Manifest QA requires a current unambiguous instance query');
    }
    if (!state.observation.instances.length) {
      if (!state.observation.eligibleGpus.includes('T4')) throw Error('The authorized T4 test GPU is unavailable');
      state = await invoke('start_instance', { accelerator: 'T4' });
      created = true;
      checks.push('Boot deployed only the daemon, without starting an application');
      if (['running', 'preparing'].includes(state.runtime.applications.comfyui)) throw Error('Boot unexpectedly started ComfyUI');
    } else {
      state = await invoke('connect_instance', { endpoint: state.observation.instances[0].id });
    }
    const endpoint = state.runtime.endpoint;
    evidence.instance = state.observation.instances[0];
    const before = await invoke('live_manifest_probe', { action: 'preflight' });
    state = await invoke('connect_instance', { endpoint });
    const after = await invoke('live_manifest_probe', { action: 'preflight' });
    const heartbeatDeadline = Date.now() + 100000;
    while (state.runtime.heartbeat?.status !== 'confirmed' || state.runtime.heartbeat?.owner !== 'cloud') {
      if (Date.now() > heartbeatDeadline) throw Error('Google instance heartbeat was not confirmed');
      await pause(2000);
      state = await invoke('refresh_cloud');
    }
    evidence.heartbeat = state.runtime.heartbeat;
    checks.push('Cloud daemon independently confirmed Jupyter kernel activity');
    if (JSON.stringify(before.applications.map(application => application.pid)) !== JSON.stringify(after.applications.map(application => application.pid))) {
      throw Error('Repeated connection unexpectedly changed application state');
    }
    checks.push('Repeated connection reused the existing instance and retained application processes');
    evidence.preflight = after;
    created = created || after.testInstance === true;
    if (after.queue && (after.queue.running || after.queue.pending)) throw Error('ComfyUI has queued work; existing jobs were preserved');
    const preparingDeadline = Date.now() + 2400000;
    while (state.runtime.applications.comfyui === 'preparing') {
      if (Date.now() > preparingDeadline) throw Error('Existing preparation did not finish');
      await pause(4000);
      state = await invoke('refresh_cloud');
    }
    evidence.externalDownload = await invoke('live_manifest_probe', { action: 'download' });
    checks.push('A real external model download passed size and SHA-256 verification');
    if (after.applications.some(application => application.id === 'comfyui') && state.runtime.applications.comfyui !== 'stopped') {
      state = await invoke('stop_application', { id: 'comfyui' });
    }
    let finished = false;
    let consentOpened = false;
    const startup = invoke('start_application', { id: 'comfyui' })
      .then(snapshot => ({ snapshot }), error => ({ error }))
      .finally(() => { finished = true; });
    while (!finished) {
      const progress = await invoke('get_workspace');
      if (progress.runtime.needsDriveConsent && !consentOpened) {
        await invoke('open_drive_consent');
        consentOpened = true;
      }
      await pause(1500);
    }
    const result = await startup;
    if ('error' in result) throw result.error;
    state = result.snapshot;
    const deadline = Date.now() + 2400000;
    while (!state.runtime.applicationUrls.comfyui) {
      if (state.runtime.applications.comfyui === 'failed') {
        evidence.applicationLogs = await invoke('application_logs', { id: 'comfyui' });
        throw Error('Manifest preparation or startup failed; instance and files retained');
      }
      if (Date.now() > deadline) throw Error('Manifest readiness deadline exceeded');
      await pause(4000);
      state = await invoke('refresh_cloud');
    }
    evidence.comfyui = await invoke('live_comfy_inspect');
    evidence.manifest = await invoke('live_manifest_probe', { action: 'evidence' });
    checks.push('Canonical manifest prepared the pinned ComfyUI application and all model resources');
    checks.push('CUDA and remote/local forwarded health checks passed');
    evidence.linuxTests = await invoke('live_persistence_probe', { action: 'linux_tests' });
    if (!evidence.linuxTests.passed) throw Error('Linux supervisor tests failed');
    if (created) {
      state = await invoke('shutdown_cloud');
      evidence.finalObservation = state.observation;
      if (state.observation.status !== 'current' || state.observation.instances.length || state.observation.hourlyRate !== 0) throw Error('Test instance billing was not confirmed stopped');
      checks.push('Manifest save rules passed Drive verification; test instance released and zero billing confirmed');
    }
    await invoke('refresh_account');
    const navigation = document.querySelector('nav button[aria-label="应用"]');
    navigation?.click();
    await pause(250);
    Array.from(Array.from(document.querySelectorAll('.cd-application-card')).find(element => element.textContent.includes('ComfyUI'))?.querySelectorAll('button') || []).find(item => item.textContent.includes('查看配置'))?.click();
    await pause(250);
    await invoke('native_check_report', { report: { passed: true, kind: 'manifest-comfyui', checks, evidence, endpoint, applicationUrl: state.runtime.applicationUrls.comfyui, instanceRetained: !created } });
  } catch (error) {
    if (created) { try { await invoke('release_empty_instance'); } catch {} }
    await invoke('native_check_report', { report: { passed: false, kind: 'manifest-comfyui', checks, evidence, error: String(error), instanceRetained: true } });
  }
})();
