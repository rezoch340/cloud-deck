// Run in the design preview console: (await import('./smoke-check.js')).checkDesign()
// Exercises only mock UI controls; never calls a cloud API.
export async function checkDesign() {
  const checks = [];
  const q = s => document.querySelector(s);
  const click = s => { const el = q(s); if (!el || el.disabled) throw Error(`Unavailable control: ${s}`); el.click(); };
  const choose = (s, value) => { const el = q(s); el.value = value; el.dispatchEvent(new Event('change', {bubbles: true})); };
  const expect = (condition, label) => { if (!condition) throw Error(label); checks.push(label); };
  const visible = () => q('#view').innerText;
  const waitFor = async predicate => {
    for (let i = 0; i < 50; i++) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw Error('Mock action did not settle in five seconds');
  };
  if (q('#dialog').open) q('#dialog').close();
  click('.nav[data-page="home"]');
  choose('#scenario', 'failed');
  click('[data-action="refresh"]');
  expect(visible().includes('HTTP 503') && !visible().includes('已关机'), '刷新后保留申请错误');
  click('.nav[data-page="logs"]');
  expect(visible().includes('原因未确认'), '错误记录不虚构资源不足原因');
  click('.nav[data-page="home"]');
  choose('#scenario', 'disconnected');
  expect(q('#footer-status').textContent.includes('计费待确认'), '连接中断不等于停止计费');
  expect(visible().includes('机器状态待确认'), '连接与实例状态分离');
  choose('#scenario', 'sync-failed');
  click('[data-action="stop"]');
  click('[data-action="confirm-stop"]');
  await waitFor(() => !q('#dialog').open);
  expect(q('#scenario').value === 'sync-failed' && q('#mini-state').textContent.includes('保留实例'), '同步失败阻止释放实例');
  choose('#scenario', 'running');
  click('[data-action="stop"]');
  click('[data-action="confirm-stop"]');
  await waitFor(() => !q('#dialog').open);
  expect(q('#scenario').value === 'off', '同步确认后才能进入已关机');
  click('[data-action="boot"]');
  expect(q('#scenario').value === 'starting', '开机先进入准备阶段');
  click('[data-action="finish-boot"]');
  click('.nav[data-page="files"]');
  choose('#inventory-mode', 'error');
  expect(visible().includes('无法读取文件清单') && !visible().includes('这个目录还没有文件'), '清单读取失败不伪装为空目录');
  choose('#inventory-mode', 'cached');
  expect(visible().includes('缓存快照') && !visible().includes('本地就绪'), '缓存文件不标记为本次就绪');
  choose('#inventory-mode', 'empty');
  expect(visible().includes('这个目录还没有文件'), '空目录有独立状态');
  choose('#inventory-mode', 'current');
  choose('#file-filter', '模型');
  const data = JSON.parse(q('#demo-data').textContent);
  expect(document.querySelectorAll('tbody tr').length === data.inventory.files.filter(f => f.kind === '模型').length, '模型行数由清单生成');
  choose('#file-filter', 'all');
  click('.nav[data-page="apps"]');
  click('[data-tab="templates"]');
  expect(document.querySelectorAll('.template-card').length === 3, '应用入口不局限于 ComfyUI');
  click('.page-top [data-action="add"]');
  q('[name="name"]').value = '端口冲突检查';
  q('[name="command"]').value = 'python app.py';
  q('[name="port"]').value = '8188';
  click('#app-form [type="submit"]');
  expect(q('#form-error').textContent.includes('端口已被'), '新增应用检查端口冲突');
  click('[data-action="close-dialog"]');
  click('.segmented [data-platform="mac"]');
  expect(document.body.dataset.platform === 'mac', 'macOS 窗口外观可切换');
  click('.segmented [data-platform="win"]');
  for (const name of ['home', 'apps', 'files', 'logs', 'settings']) {
    click(`.nav[data-page="${name}"]`);
    expect(document.documentElement.scrollWidth <= innerWidth, `${name} 无全页横向溢出`);
  }
  click('.nav[data-page="home"]');
  return {passed: checks.length, checks};
}

// On onboarding.html: (await import('./smoke-check.js')).checkOnboarding()
export async function checkOnboarding() {
  const checks = [];
  const q = selector => document.querySelector(selector);
  const click = selector => {
    const el = q(selector);
    if (!el || el.disabled) throw Error(`Unavailable control: ${selector}`);
    el.click();
  };
  const choose = (selector, value) => {
    const el = q(selector);
    el.value = value;
    el.dispatchEvent(new Event('change', {bubbles: true}));
  };
  const expect = (condition, label) => {
    if (!condition) throw Error(label);
    checks.push(label);
  };
  const text = () => q('#page').innerText;
  const billing = () => q('#billing').textContent;
  const waitFor = async predicate => {
    for (let i = 0; i < 50; i++) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw Error('Mock preparation did not finish');
  };
  choose('#scene', 'welcome');
  expect(!q('input[type="password"]') && billing().includes('不会申请'), '首次打开不收集密码、不申请实例');
  click('[data-action="login"]');
  expect(q('#dialog').open && q('#dialog').innerText.includes('不是 Google 登录页面'), '浏览器示意明确标注，不伪装真实登录');
  click('[data-action="deny-browser"]');
  expect(q('#scene').value === 'cancelled' && billing().includes('不会申请'), '取消授权不会启动实例');
  click('[data-action="login"]');
  click('[value="studio"][name="demo-account"]');
  click('[data-action="accept-browser"]');
  expect(text().includes('studio@example.com'), '可以选择不同 Google 账号');
  expect(text().includes('Colab 服务访问') && text().includes('可用计算权益'), '身份、Colab 访问和权益分别确认');
  choose('#scene', 'network');
  expect(!q('[data-action="setup"]') && text().includes('不把查询失败当成余额为零'), '查询失败保留身份，阻止启动且不虚构零余额');
  click('[data-action="retry-check"]');
  expect(q('[data-action="setup"]') && billing().includes('不会申请'), '重试检查成功仍不申请实例');
  choose('#scene', 'restricted');
  expect(!q('[data-action="setup"]') && text().includes('免费档'), '权益不足不会进入启动流程');
  choose('#scene', 'account');
  click('[data-action="setup"]');
  choose('#gpu', 'L4');
  click('[data-action="start"]');
  expect(q('#dialog').innerText.includes('L4') && q('#dialog').innerText.includes('等待云盘授权'), '启动确认使用所选 GPU，说明准备和授权期间计费');
  click('#dialog [data-action="close"]');
  expect(billing().includes('不会申请'), '取消启动确认不改变实例状态');
  click('[data-action="start"]');
  click('[data-action="confirm-start"]');
  expect(q('#scene').value === 'drive' && billing().includes('实例计费中'), '分配后等待 Drive 授权，明确计费中');
  expect(q('#status-items').textContent.includes('应用：未启动'), '保存位置未连接时不启动应用');
  click('[data-action="drive-browser"]');
  click('[data-action="deny-browser"]');
  expect(q('#scene').value === 'drive-error' && billing().includes('实例计费中'), 'Drive 授权失败保留实例与计费状态');
  click('[data-action="drive-browser"]');
  click('[data-action="accept-browser"]');
  await waitFor(() => q('#scene').value === 'ready');
  expect(text().includes('L4 GPU') && q('#status-items').textContent.includes('应用：就绪'), 'Drive 检查后准备应用，保留用户所选 GPU');
  click('[data-action="account-menu"]');
  expect(q('#dialog').innerText.includes('先确认机器状态') && !q('#dialog [data-action="switch-account"]'), '运行中的账号不能直接切换或退出');
  click('#dialog [data-action="close"]');
  choose('#scene', 'expired');
  expect(billing().includes('计费待确认') && !billing().includes('停止'), '授权失效不等于实例已释放或停止计费');
  click('[data-action="reauth"]');
  expect(document.querySelectorAll('[name="demo-account"]').length === 1, '恢复已有实例时引导使用原账号');
  click('[data-action="deny-browser"]');
  expect(q('#scene').value === 'expired' && billing().includes('计费待确认'), '取消重新授权仍保留已有实例未知状态');
  click('[data-action="reauth"]');
  click('[data-action="accept-browser"]');
  expect(q('#scene').value === 'ready', '重新授权后接回原工作空间');
  choose('#scene', 'drive');
  click('[data-action="drive-browser"]');
  click('[data-action="accept-browser"]');
  choose('#scene', 'welcome');
  await new Promise(resolve => setTimeout(resolve, 1500));
  expect(q('#scene').value === 'welcome', '切换演示场景取消旧准备任务，迟到结果不覆盖新状态');
  choose('#scene', 'timeout');
  expect(text().includes('旧页面的返回不会继续'), '超时说明旧授权请求失效');
  click('.platform [data-platform="mac"]');
  expect(document.body.dataset.platform === 'mac' && q('#original-link').href.includes('platform=mac'), 'macOS 窗口与后续工作台链接保持一致');
  click('.platform [data-platform="win"]');
  for (const scene of ['welcome', 'waiting', 'account', 'setup', 'drive', 'drive-error', 'ready', 'restricted', 'network', 'cancelled', 'timeout', 'expired']) {
    choose('#scene', scene);
    expect(document.documentElement.scrollWidth <= innerWidth && q('#main').scrollWidth <= q('#main').clientWidth + 1, `${scene} 没有横向溢出`);
  }
  choose('#scene', 'welcome');
  return {passed: checks.length, checks};
}
