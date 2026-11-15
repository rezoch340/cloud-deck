// UI prototype only: the JSON is an example directory snapshot, not a live Drive scan.
let inventory = null;
let inventoryStatus = 'loading';
let inventoryError = '';
let inventoryRequest = 0;

function formatBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KiB', 'MiB', 'GiB', 'TiB'];
  let value = bytes / 1024, unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; }
  return `${value.toFixed(2)} ${units[unit]}`;
}

function validateInventory(data) {
  if (!data || typeof data.root !== 'string' || !Array.isArray(data.files)) {
    throw new Error('模型清单格式不正确');
  }
  const paths = new Set();
  for (const file of data.files) {
    if (!file || typeof file.path !== 'string' || !file.path.trim() ||
        !Number.isSafeInteger(file.size) || file.size < 0 || paths.has(file.path)) {
      throw new Error('模型清单中的文件名或大小无效');
    }
    paths.add(file.path);
  }
  return data;
}

async function loadInventory() {
  const request = ++inventoryRequest;
  const demo = $('inventory-demo').value;
  inventoryStatus = 'loading';
  inventory = null;
  renderInventory();
  if (demo === 'loading') return;
  try {
    if (demo === 'error') throw new Error('保存目录暂时无法读取，请重试');
    const response = await fetch('inventory-preview.json', {
      cache: 'no-store', signal: AbortSignal.timeout(8000)
    });
    if (!response.ok) throw new Error(`读取清单失败（HTTP ${response.status}）`);
    const data = validateInventory(await response.json());
    if (request !== inventoryRequest) return;
    inventory = demo === 'empty'
      ? {...data, files: [], workflowCount: 0, instanceSnapshots: {}}
      : data;
    inventoryStatus = 'loaded';
  } catch (error) {
    if (request !== inventoryRequest) return;
    inventoryStatus = 'error';
    inventoryError = location.protocol === 'file:'
      ? '请通过本地预览服务打开页面后读取清单'
      : error.message;
  }
  renderInventory();
}

function inventoryElement(tag, className, text) {
  const el = document.createElement(tag);
  el.className = className;
  if (text !== undefined) el.textContent = text;
  return el;
}

function renderInventory() {
  const list = $('models');
  list.replaceChildren();
  list.setAttribute('aria-busy', String(inventoryStatus === 'loading'));
  $('refresh-models').disabled = inventoryStatus === 'loading';
  $('retry-models').hidden = inventoryStatus !== 'error';
  $('inventory-status').parentElement.classList.toggle('error', inventoryStatus === 'error');
  $('copy-progress').hidden = true;
  $('copy-progress').removeAttribute('aria-valuenow');
  $('copy-detail').textContent = '读取保存目录后计算大小';
  $('model-count').textContent = inventoryStatus === 'loading' ? '…' : '—';
  $('workflow-count').textContent = '工作流数量待读取';
  if (current === 'ready') $('state-description').textContent = states.ready.description;
  if (current === 'copy') $('state-description').textContent = '正在读取模型清单与本次拷贝进度…';

  if (!inventory) {
    $('model-location').textContent = inventoryStatus === 'error' ? '状态未知' : '读取中';
    $('inventory-source').textContent = '来源：配置的模型保存目录';
    $('inventory-status').textContent = inventoryStatus === 'error' ? inventoryError : '正在读取模型清单…';
    list.append(inventoryElement('p', 'inventory-empty', inventoryStatus === 'error'
      ? '读取失败，不能确认已保存哪些模型。'
      : '正在读取目录中的文件名、大小和准备状态。'));
    return;
  }

  const files = inventory.files;
  const total = files.reduce((sum, file) => sum + file.size, 0);
  const snapshots = inventory.instanceSnapshots?.[current] || {};
  const isReady = file => snapshots[file.path]?.status === 'ready' &&
    snapshots[file.path]?.copiedBytes === file.size && file.size > 0;
  const readyCount = files.filter(isReady).length;
  const cached = ['off', 'auth', 'error'].includes(current);
  $('model-count').textContent = files.length;
  $('inventory-source').textContent = inventory.root;
  $('model-location').textContent = cached
    ? `${files.length} 个文件 · ${formatBytes(total)}`
    : `${readyCount} / ${files.length} 本地就绪 · ${formatBytes(total)}`;
  const observed = inventory.observedAt ? new Date(inventory.observedAt) : null;
  const timestamp = observed && !Number.isNaN(observed.getTime())
    ? observed.toLocaleString('zh-CN', {hour12: false}) : '时间未提供';
  $('inventory-status').textContent = inventory.preview
    ? (cached ? '示例：上次读取的目录快照，非实时清单。' : '示例目录与拷贝状态，尚未连接真实云盘。')
    : `${cached ? '上次读取' : '读取时间'}：${timestamp}${cached ? ' · 待重新核对' : ''}`;
  if (Number.isSafeInteger(inventory.workflowCount) && inventory.workflowCount >= 0) {
    $('workflow-count').textContent = `${inventory.workflowCount} 个工作流${cached ? ' · 上次读取' : ''}`;
  }

  for (const file of files) {
    const parts = file.path.split('/');
    const filename = parts.pop();
    const category = parts.join('/') || 'models';
    const state = snapshots[file.path];
    const localReady = isReady(file);
    const label = localReady ? '本地就绪' : state?.status === 'copying' ? '拷贝中'
      : cached ? '待核对' : state?.status === 'pending' ? '待拷贝' : '未核对';
    const row = inventoryElement('div', 'model');
    row.dataset.path = file.path;
    const icon = inventoryElement('div', 'model-icon', category === 'upscale_models' ? '放大' : '模型');
    icon.setAttribute('aria-hidden', 'true');
    const info = inventoryElement('div', 'model-info');
    const name = inventoryElement('p', 'model-name', filename);
    name.title = file.path;
    info.append(name, inventoryElement('p', 'model-file', category));
    const status = inventoryElement('span', `model-state${localReady ? ' local-ready' : ''}`, label);
    status.title = cached ? '保存目录的历史记录，当前尚未核对' : label;
    row.append(icon, info, inventoryElement('span', 'model-size', formatBytes(file.size)), status);
    list.append(row);
  }
  if (!files.length) {
    list.append(inventoryElement('p', 'inventory-empty', '保存目录中还没有模型。保存模型后刷新即可显示。'));
  }
  $('copy-detail').textContent = files.length ? `${files.length} 个文件，共 ${formatBytes(total)}` : '目录中暂无模型';
  if (current === 'ready') {
    $('state-description').textContent = `ComfyUI 已启动；${readyCount} / ${files.length} 个模型已核对本地拷贝。`;
  }
  if (current === 'copy' && files.length) {
    const reported = files.every(file => Number.isFinite(snapshots[file.path]?.copiedBytes));
    if (reported && total > 0) {
      const copied = files.reduce((sum, file) => sum + Math.max(0, Math.min(file.size, snapshots[file.path].copiedBytes)), 0);
      const percent = Math.round(copied / total * 100);
      $('copy-progress').hidden = false;
      $('copy-progress').setAttribute('aria-valuenow', percent);
      $('copy-progress').querySelector('.progress-fill').style.width = `${percent}%`;
      $('state-description').textContent = `${formatBytes(copied)} / ${formatBytes(total)} 已拷贝，完成后自动启动。`;
    }
  }
}
