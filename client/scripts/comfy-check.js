// Explicit live QA only. Uses the product commands and keeps any application data.
(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const checks = [];
  const pause = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  let started = false;
  let allocated = false;
  try {
    for (let attempt = 0; attempt < 100 && !document.querySelector('nav'); attempt++) await pause(100);
    let state = await invoke('refresh_account');
    if (!state.account || state.observation.status !== 'current') throw Error(state.observation.error || 'Sign in before the live ComfyUI check');
    const available = state.observation.eligibleGpus;
    const accelerator = ['T4', 'L4', 'G4', 'A100', 'H100'].find(value => available.includes(value));
    if (!state.observation.instances.length && !accelerator) throw Error('No eligible GPU is confirmed for this account');
    if (state.observation.instances.length && state.observation.instances[0].accelerator === 'NONE') throw Error('The existing CPU instance was preserved; ComfyUI requires a GPU');
    allocated = state.observation.instances.length === 0;
    state = allocated
      ? await invoke('start_instance', { accelerator })
      : await invoke('connect_instance', { endpoint: state.observation.instances[0].id });
    checks.push('Real GPU instance connected and bundled daemon deployed');
    await invoke('refresh_account');
    const navigation = document.querySelector('nav button[aria-label="应用"]');
    navigation.click();
    await pause(150);
    const card = Array.from(document.querySelectorAll('.cd-application-card')).find(element => element.textContent.includes('ComfyUI'));
    card?.querySelector('button')?.click();
    await pause(150);
    const resumed = ['preparing', 'running'].includes(state.runtime.applications.comfyui);
    if (!resumed) {
      let finished = false;
      let consentOpened = false;
      const startup = invoke('start_application', { id: 'comfyui' })
        .then(snapshot => ({ snapshot }), error => ({ error }))
        .finally(() => { finished = true; });
      try {
        // Startup now waits for browser consent and resumes without another command.
        while (!finished) {
          const progress = await invoke('get_workspace');
          if (progress.runtime.needsDriveConsent && !consentOpened) {
            await invoke('open_drive_consent');
            consentOpened = true;
          }
          await pause(1500);
        }
      } finally {
        await startup;
      }
      const result = await startup;
      if ('error' in result) throw result.error;
      state = result.snapshot;
    }
    started = ['preparing', 'running', 'failed'].includes(state.runtime.applications.comfyui);
    checks.push(resumed ? 'Existing supervised ComfyUI resumed without restarting the application' : 'Drive mount gate passed before managed ComfyUI preparation');
    const readyDeadline = Date.now() + 2400000;
    while (!state.runtime.applicationUrls.comfyui) {
      if (state.runtime.applications.comfyui === 'failed') {
        throw Error('ComfyUI preparation or startup failed; the instance and its files were preserved');
      }
      if (Date.now() > readyDeadline) throw Error('ComfyUI readiness deadline exceeded; the instance and its files were preserved');
      await pause(4000);
      state = await invoke('refresh_cloud');
    }
    checks.push('Managed ComfyUI passed remote and loopback-forwarded /system_stats health checks');
    const evidence = await invoke('live_comfy_inspect');
    checks.push('Actual pinned Git revision, CUDA device, Drive mount and configured model sizes verified');
    await invoke('refresh_account');
    await invoke('native_check_report', { report: { passed: true, kind: 'live-comfyui', checks, evidence, endpoint: state.runtime.endpoint, applicationUrl: state.runtime.applicationUrls.comfyui, instanceRetained: true, synchronizationVerified: false } });
  } catch (error) {
    let releasedUnusedInstance = false;
    if (allocated && !started) {
      // Product command refuses release if any application may have written data.
      try { await invoke('release_empty_instance'); releasedUnusedInstance = true; } catch {}
    }
    await invoke('refresh_account').catch(() => {});
    await invoke('native_check_report', { report: { passed: false, kind: 'live-comfyui', checks, error: String(error), releasedUnusedInstance, instanceStateRequiresConfirmation: !releasedUnusedInstance } });
  }
})();
