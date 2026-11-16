// Run in the Vite preview: (await import('/ui-smoke-check.js')).checkClientUI()
// Only operates on browser preview configuration; never invokes native/cloud APIs.
import { getWorkspace, desktop } from './src/api.ts';
import manifest from '../manifest.json';

export async function checkClientUI() {
  if (desktop) throw Error('This check is only for the browser preview.');
  const results = [];
  const q = selector => document.querySelector(selector);
  const waitFor = async predicate => {
    for (let i = 0; i < 80; i++) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 35));
    }
    throw Error('UI condition did not become true');
  };
  const settle = async () => { await Promise.resolve(); await Promise.resolve(); };
  const check = (condition, message) => { if (!condition) throw Error(message); results.push(message); };
  const click = async selector => {
    const element = typeof selector === 'string' ? q(selector) : selector;
    if (!element || element.disabled) throw Error(`Control unavailable: ${selector}`);
    element.click(); await settle();
  };
  const button = (text, root = document) => Array.from(root.querySelectorAll('button')).find(b => b.textContent.trim() === text);
  async function fill(name, value) {
    let input = q(`[role="dialog"] [name="${name}"]`);
    if (input?.type === 'hidden') {
      const label = name === 'remotePort' ? '远端服务端口' : '本机访问端口';
      input = Array.from(document.querySelectorAll('[role="dialog"] .number-field')).find(field => field.textContent.includes(label))?.querySelector('input:not([type="hidden"])');
    }
    if (!input) throw Error('Input unavailable: ' + name);
    input.focus();
    const prototype = input instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, 'value').set.call(input, String(value));
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
    await new Promise(resolve => setTimeout(resolve, 20));
    input.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: document.body }));
    input.blur();
    await new Promise(resolve => setTimeout(resolve, 20));
  }
  await waitFor(() => q('nav button[aria-label="应用"]'));
  if (q('[role="dialog"]')) { await click('[aria-label="关闭对话框"]'); await waitFor(() => !q('[role="dialog"]')); }
  const before = await getWorkspace();
  const name = '界面检查-' + Date.now();
  let createdId = null;
  await click('nav button[aria-label="应用"]');
  check(q('.cd-page-header h1').textContent === '应用', '主要导航切换到应用页');
  const builtin = Array.from(document.querySelectorAll('.cd-application-card')).find(card => card.textContent.includes('ComfyUI'));
  await click(button('查看配置', builtin));
  check(q('.cd-page-content').textContent.includes(manifest.comfyui.version), '内置版本来自现有清单');
  check(q('.cd-page-content').textContent.includes(`${manifest.plugins.length} 个插件记录`), '插件数量来自现有清单');
  check(q('.cd-page-content').textContent.includes('cloud_bootstrap.py'), '内置配方复用现有准备脚本');
  check(!button('编辑配置'), '内置固定配方不能被自定义配置覆盖');
  check(['启动应用', '打开应用'].every(text => button(text)?.disabled), '未连接和未通过健康检查时不能启动或打开应用');
  check(q('.cd-page-content').textContent.includes('未确认'), '应用未知状态不会显示运行或停止结论');
  await click('nav button[aria-label="文件与同步"]');
  check(q('tbody').querySelectorAll('tr').length === manifest.models.length, '模型下载配置按清单生成');
  check(Array.from(q('tbody').querySelectorAll('tr')).every(row => row.textContent.includes('下载配置')), '模型配置不伪装成已经下载或保存');
  check(button('保存并校验')?.disabled && !q('.cd-page-content').textContent.includes('Drive 服务端已校验'), '未连接时不能保存，也不伪造服务端校验结果');
  await click('nav button[aria-label="应用"]');
  await click(button('添加应用'));
  await waitFor(() => q('[role="dialog"]'));
  check(!!q('[role="dialog"]').getAttribute('aria-labelledby') && q('[role="dialog"]').textContent.includes('添加应用'), 'HeroUI 弹窗具有可访问的标题与模态语义');
  const heights = Array.from(document.querySelectorAll('[role="dialog"] .input,[role="dialog"] .select__trigger,[role="dialog"] .number-field__group')).map(el => Math.round(el.getBoundingClientRect().height));
  check(Math.max(...heights) - Math.min(...heights) <= 1, '文本、选择和数字输入高度一致');
  await fill('name', name);
  await fill('remotePort', 8188);
  await click('[role="dialog"] button[type="submit"]');
  await waitFor(() => q('[role="dialog"] [role="alert"]'));
  check(q('[role="dialog"] [role="alert"]').textContent.includes('端口'), '重复端口会阻止保存并显示错误');
  const errorRect = q('[role="dialog"] [role="alert"]').getBoundingClientRect();
  const dialogRect = q('[role="dialog"]').getBoundingClientRect();
  check(errorRect.top >= dialogRect.top && errorRect.bottom <= dialogRect.bottom, '保存错误始终显示在可见弹窗范围');
  let remotePort = 28000, localPort = 29000;
  while (before.apps.some(app => app.remotePort === remotePort)) remotePort++;
  while (before.apps.some(app => app.localPort === localPort)) localPort++;
  await fill('remotePort', remotePort);
  await fill('localPort', localPort);
  await click('[role="dialog"] button[type="submit"]');
  await waitFor(() => !q('[role="dialog"]'));
  const saved = await getWorkspace();
  const created = saved.apps.find(app => app.name === name);
  check(!!created && created.remotePort === remotePort && created.localPort === localPort, '应用配置真实保存到预览存储');
  createdId = created.id;
  const card = Array.from(document.querySelectorAll('.cd-application-card')).find(c => c.textContent.includes(name));
  await click(button('查看配置', card));
  await click(button('编辑配置'));
  await fill('name', name + '-已编辑');
  await click('[role="dialog"] button[aria-haspopup="listbox"]');
  await waitFor(() => q('[role="option"]'));
  await click(Array.from(document.querySelectorAll('[role="option"]')).find(item => item.textContent.includes('一次性脚本任务')));
  check(!q('[role="dialog"] [name="remotePort"]'), '切换为任务后不再要求服务端口');
  await click('[role="dialog"] button[type="submit"]');
  await waitFor(() => !q('[role="dialog"]'));
  const edited = (await getWorkspace()).apps.find(app => app.id === createdId);
  check(edited.kind === 'task' && edited.remotePort === null, '编辑保留应用标识并更新任务类型');
  const editedCard = Array.from(document.querySelectorAll('.cd-application-card')).find(c => c.textContent.includes(name + '-已编辑'));
  await click(button('查看配置', editedCard));
  await click(button('移除本地配置'));
  await click(button('移除配置', q('[role="dialog"]')));
  await waitFor(() => !q('[role="dialog"]'));
  check((await getWorkspace()).apps.length === before.apps.length, '只删除本次测试配置，保留其他应用');
  await click('nav button[aria-label="活动记录"]');
  check(q('.cd-page-content').textContent.includes(name), '配置变更写入活动记录');
  for (const label of ['应用', '文件与同步', '活动记录']) {
    await click(`nav button[aria-label="${label}"]`);
    check(document.documentElement.scrollWidth <= innerWidth && q('.cd-page-content').scrollWidth <= q('.cd-page-content').clientWidth + 1, label + ' 没有窗口横向溢出');
  }
  await click('button[aria-label="设置"]');
  check(!q('.cd-page-content').textContent.includes('已同步'), '未接入同步时不显示已保存结论');
  await click('nav button[aria-label="应用"]');
  return { passed: results.length, checks: results, viewport: [innerWidth, innerHeight] };
}
