import type { Recipe, Snapshot } from '../domain.ts';
import { bytes } from '../domain.ts';
import { Badge, Button, DataTable, Notice, Panel, SelectInput } from '../components/ui.tsx';
import { ManifestLifecycle } from '../components/ManifestLifecycle.tsx';

export function Files({
  apps,
  app,
  select,
  runtime,
  lastSaveVerification,
  busy,
  save,
}: {
  apps: Recipe[];
  app: Recipe;
  select: (applicationId: string) => void;
  runtime: Snapshot['runtime'];
  lastSaveVerification?: Snapshot['lastSaveVerification'];
  busy: boolean;
  save: () => void;
}) {
  const files = runtime.storage.files?.filter((file) => file.applicationId === app.id) || [];
  const previous = lastSaveVerification?.storage;
  const previousFiles = previous?.files?.filter((file) => file.applicationId === app.id) || [];
  const showPrevious =
    previousFiles.length > 0 &&
    (runtime.connection !== 'connected' ||
      runtime.synchronization !== 'verified' ||
      previous?.receiptId !== runtime.storage.receiptId);
  const synchronization = {
    unobserved: '本次未查询',
    pending: '待同步',
    syncing: '正在保存与校验',
    verified: runtime.storage.method === 'no_save_rules' ? '没有待保存文件' : 'Drive 服务端已校验',
    failed: '保存失败，实例保留',
  }[runtime.synchronization];
  return (
    <div className="cd-stack">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <SelectInput
          label="应用"
          className="w-64 max-w-full"
          value={app.id}
          onChange={select}
          options={apps.map((application) => ({ value: application.id, label: application.name }))}
        />
      </div>
      <Panel
        title="文件保存"
        actions={
          <Badge
            tone={
              runtime.synchronization === 'verified'
                ? 'good'
                : runtime.synchronization === 'failed'
                  ? 'bad'
                  : 'neutral'
            }
          >
            {synchronization}
          </Badge>
        }
      >
        {runtime.storage.error && <Notice tone="bad">{runtime.storage.error}</Notice>}
        <p className="text-sm text-muted">
          {runtime.storage.observedAt
            ? `上次观测：${new Date(runtime.storage.observedAt).toLocaleString('zh-CN')}。`
            : runtime.connection === 'connected'
              ? '当前实例尚未执行保存校验。'
              : '当前未连接实例，无法查询本次文件状态；这不代表云盘中没有已保存文件。'}
          {runtime.storage.method === 'drive_api_md5'
            ? '逐文件核对 Google Drive 服务端大小与 MD5。'
            : runtime.storage.method === 'no_save_rules'
              ? '已部署的应用没有配置保存规则，无文件需要同步。'
              : ''}
        </p>
        <div className="my-4 flex flex-wrap items-center gap-4">
          <Button
            variant="primary"
            onPress={save}
            isDisabled={
              busy || runtime.connection !== 'connected' || runtime.synchronization === 'syncing'
            }
          >
            保存并校验
          </Button>
          {files.length > 0 && (
            <span className="text-sm text-muted">
              {files.length} 个文件 · {bytes(files.reduce((total, file) => total + file.size, 0))}
            </span>
          )}
        </div>
        {files.length > 0 && (
          <div
            className="max-h-80 overflow-y-auto"
            role="region"
            aria-label="保存文件清单滚动区域"
            tabIndex={0}
          >
            <DataTable
              label="已校验保存文件"
              columns={['文件', '大小', 'SHA-256']}
              rows={files.map((file) => ({
                id: file.path,
                cells: [file.path, bytes(file.size), <code>{file.sha256.slice(0, 16)}…</code>],
              }))}
            />
          </div>
        )}
      </Panel>
      {showPrevious && previous && (
        <Panel title="上次保存记录" actions={<Badge tone="neutral">历史校验记录</Badge>}>
          <p className="mb-4 text-sm text-muted">
            {new Date(previous.observedAt!).toLocaleString('zh-CN')}，{previousFiles.length} 个文件
            · {bytes(previousFiles.reduce((total, file) => total + file.size, 0))}。 当时已通过
            Drive 服务端大小与 MD5 校验；之后的文件变动需要重新查询。
          </p>
          <div
            className="max-h-80 overflow-y-auto"
            role="region"
            aria-label="历史保存文件清单"
            tabIndex={0}
          >
            <DataTable
              label="上次已校验文件"
              columns={['文件', '大小', 'SHA-256']}
              rows={previousFiles.map((file) => ({
                id: file.path,
                cells: [file.path, bytes(file.size), <code>{file.sha256.slice(0, 16)}…</code>],
              }))}
            />
          </div>
        </Panel>
      )}
      {app.manifest && <ManifestLifecycle app={app} />}
      <Panel title="保存目录">
        <dl className="cd-properties">
          <div>
            <dt>实例目录</dt>
            <dd>
              {app.saveDirectories.length
                ? app.saveDirectories.map((entry) => (
                    <code key={entry}>
                      {entry}
                      <br />
                    </code>
                  ))
                : '尚未配置'}
            </dd>
          </div>
          <div>
            <dt>云盘目标</dt>
            <dd>
              <code>{app.storageDirectory}</code>
            </dd>
          </div>
        </dl>
        <p className="mt-4 text-xs text-muted">
          保存会先停止受管应用，再向 Drive
          复制配置目录。已有不同内容会保留备份，不传播删除；通过服务端校验后才能关机。
        </p>
      </Panel>
    </div>
  );
}
