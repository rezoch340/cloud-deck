// Explicit live QA only. Saves and releases the single existing VM; never allocates one.
(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  const checks = [];
  const waitFor = async (read, timeout = 1200000) => {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      const result = await read();
      if (result) return result;
      await pause(1000);
    }
    throw Error('Shutdown deadline exceeded; query VM and billing state before further action');
  };
  let endpoint;
  try {
    await waitFor(() => document.querySelector('nav'), 15000);
    let state;
    for (let attempt = 0; attempt < 3; attempt++) {
      try { state = await invoke('refresh_account'); } catch (error) { if (attempt === 2) throw error; }
      if (state?.observation.status === 'current') break;
      if (attempt < 2) await pause(2000 * (attempt + 1));
    }
    if (state?.observation.status !== 'current') throw Error(state?.observation.error || 'Account query failed');
    if (state.observation.instances.length !== 1) throw Error('Exactly one existing VM is required; none was allocated');
    endpoint = state.observation.instances[0].id;
    await invoke('connect_instance', { endpoint: state.observation.instances[0].id });
    const linuxChecks = await invoke('live_persistence_probe', { action: 'linux_tests' });
    if (!linuxChecks.passed) throw Error('Linux checks failed: ' + linuxChecks.output);
    checks.push('Latest Linux supervisor checks passed');
    document.querySelector('nav button[aria-label="云端工作台"]').click();
    const button = text => Array.from(document.querySelectorAll('button')).find(item => item.textContent.trim() === text && !item.disabled);
    await waitFor(() => button('保存并关闭实例'), 10000);
    button('保存并关闭实例').click();
    await waitFor(() => document.querySelector('[role="dialog"]'), 10000);
    const confirm = Array.from(document.querySelectorAll('[role="dialog"] button')).find(item => item.textContent.trim() === '保存并关机');
    if (!confirm) throw Error('Save-and-shutdown confirmation is missing');
    confirm.click();
    state = await waitFor(async () => {
      const next = await invoke('get_workspace');
      if (next.runtime.phase === 'error') throw Error(next.runtime.message);
      return next.observation.status === 'current' && next.observation.instances.length === 0 && next.observation.hourlyRate === 0 && !next.runtime.endpoint ? next : null;
    });
    checks.push('Native save-and-shutdown control stopped writers, saved configured files and rechecked Drive server contents before release');
    checks.push('Fresh Colab observation confirmed no assignments and zero hourly billing');
    await pause(500);
    await invoke('native_check_report', { report: { passed: true, kind: 'live-save-shutdown', checks, linuxChecks, endpoint, releaseExecuted: true, billingConfirmedOff: true, instanceRetained: false, finalObservation: { status: state.observation.status, hourlyRate: state.observation.hourlyRate, instances: state.observation.instances, observedAt: state.observation.observedAt } } });
  } catch (error) {
    await invoke('refresh_account').catch(() => {});
    await invoke('native_check_report', { report: { passed: false, kind: 'live-save-shutdown', checks, endpoint, error: String(error), instanceAndBillingRequireConfirmation: true } });
  }
})();
