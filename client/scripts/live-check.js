// Live QA only: requires a native-smoke build AND explicit CLOUD_DECK_LIVE_CHECK=1.
// The user completes Google's consent in the system browser. No token is exported.
(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const checks = [];
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
  const waitFor = async (predicate, milliseconds = 10000) => {
    const deadline = Date.now() + milliseconds;
    while (Date.now() < deadline) {
      const result = await predicate();
      if (result) return result;
      await sleep(150);
    }
    throw Error('Live check did not reach the expected state before its deadline');
  };
  let deployment;
  try {
    await waitFor(() => document.querySelector('nav button[aria-label="应用"]'));
    const initial = await invoke('get_workspace');
    const after = Math.max(0, ...initial.events.filter(event => event.category === 'account').map(event => event.time));
    if (initial.account) {
      // Resume after a local fix using the stored refresh token, without new consent.
      const restored = await invoke('refresh_account');
      if (restored.observation.status !== 'current') throw Error(restored.observation.error || 'Colab read-only queries did not pass');
      checks.push('Saved Rust OAuth credential refreshed and Google identity verified');
    } else {
      const login = await waitFor(() => Array.from(document.querySelectorAll('button')).find(button => button.textContent.trim() === '使用 Google 账号继续' && !button.disabled));
      login.click();
      await waitFor(async () => {
        const state = await invoke('get_workspace');
        const failure = state.events.find(event => event.category === 'account' && event.level === 'error' && event.time > after);
        if (failure) throw Error(failure.message);
        const completed = state.events.some(event => event.category === 'account' && event.time > after && event.message.includes('Google 账号已连接'));
        if (completed && state.observation.status === 'error') {
          checks.push('Real Rust browser OAuth and secure credential storage passed');
          throw Error(state.observation.error || 'Colab read-only queries did not pass');
        }
        return completed && state.account?.status === 'connected' && state.observation.status === 'current';
      }, 660000);
      checks.push('Real Rust browser OAuth and secure credential storage passed');
    }
    checks.push('Colab read-only queries passed');
    deployment = await invoke('live_deployment_check');
    checks.push('Bundled cloud resource hashes verified on the real instance');
    checks.push('Native SSH over protected WebSocket completed a task and retained its log');
    checks.push('Real HTTP service health passed through the built-in loopback forward');
    checks.push('Validation processes stopped; only a newly created test instance was released');
    document.querySelector('button[aria-label="设置"]').click();
    await sleep(100);
    const refresh = Array.from(document.querySelectorAll('header button')).find(button => button.textContent.includes('刷新状态'));
    refresh?.click();
    await sleep(200);
    await waitFor(() => !Array.from(document.querySelectorAll('header button')).some(button => button.textContent.includes('正在处理')), 90000);
    document.querySelector('nav button[aria-label="活动记录"]').click();
    await sleep(150);
    await invoke('native_check_report', { report: { passed: true, kind: 'live-oauth-and-deployment', checks, deployment } });
  } catch (error) {
    await invoke('native_check_report', { report: { passed: false, kind: 'live-oauth-and-deployment', checks, deployment, error: String(error), cloudStateRequiresConfirmation: true } });
  }
})();
