import type { ReactNode } from 'react';
import type { AppKind, Recipe, Snapshot } from '../domain.ts';
import { ManifestLifecycle } from '../components/ManifestLifecycle.tsx';
import { ApplicationDescription } from '../components/ApplicationDescription.tsx';
import { Badge, Button, Notice, Panel } from '../components/ui.tsx';
import { Icon } from '../components/Icon.tsx';
import { readinessDescription } from '../readiness.ts';

const applicationIcon = (app: Recipe) =>
  app.builtin ? 'node' : app.kind === 'task' ? 'code' : 'service';
export function ApplicationCard({ app, open }: { app: Recipe; open: (app: Recipe) => void }) {
  return (
    <Panel className="cd-application-card">
      <div className="flex items-center gap-4">
        <div className={`cd-app-icon ${app.builtin ? '' : 'neutral'}`}>
          <Icon name={applicationIcon(app)} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="font-semibold">{app.name}</h3>
            {app.builtin && <Badge tone="accent">内置配置</Badge>}
          </div>
          <p className="mt-1 text-xs text-muted">
            {app.description || (app.kind === 'task' ? '一次性任务' : '长期运行的服务')}
          </p>
        </div>
        <Button variant="outline" onPress={() => open(app)}>
          查看配置
          <Icon name="chevron" />
        </Button>
      </div>
    </Panel>
  );
}
export function Applications({
  apps,
  open,
  add,
  addManifest,
}: {
  apps: Recipe[];
  open: (app: Recipe) => void;
  add: (kind: AppKind) => void;
  addManifest: () => void;
}) {
  return (
    <div className="cd-stack">
      <div className="cd-stack-sm">
        {apps.map((app) => (
          <ApplicationCard key={app.id} app={app} open={open} />
        ))}
      </div>
      <div className="cd-two-columns">
        <Panel title="Python / 脚本任务" description="保存一次性任务的命令与产物目录。">
          <div className="flex items-center justify-between gap-3">
            <Icon name="code" />
            <Button variant="outline" onPress={() => add('task')}>
              添加任务
            </Button>
          </div>
        </Panel>
        <Panel title="自定义 Web 服务" description="配置入口、健康检查和需要保留的文件。">
          <div className="flex items-center justify-between gap-3">
            <Icon name="service" />
            <Button variant="outline" onPress={() => add('service')}>
              添加服务
            </Button>
          </div>
        </Panel>
      </div>
      <Panel
        title="应用清单"
        description="用 manifest.json 定义资源优先级、启动恢复与命令、关机保存。"
      >
        <Button variant="outline" onPress={addManifest}>
          添加应用清单
        </Button>
      </Panel>
      <Notice>
        内置 ComfyUI 优先使用项目清单。添加应用只保存配置，不会下载模型、分配机器或执行命令。
      </Notice>
    </div>
  );
}
export function ApplicationRuntime({
  app,
  runtime,
  busy,
  start,
  stop,
  open,
}: {
  app: Recipe;
  runtime: Snapshot['runtime'];
  busy: boolean;
  start: () => void;
  stop: () => void;
  open: () => void;
}) {
  const state = runtime.applications[app.id] || 'unknown';
  const labels = {
    unknown: '未确认',
    preparing: '准备中',
    running: '运行中',
    stopped: '已停止',
    failed: '失败',
  };
  const connected = runtime.connection === 'connected';
  return (
    <Panel
      title="云端运行"
      actions={
        <Badge tone={state === 'failed' ? 'bad' : state === 'running' ? 'good' : 'neutral'}>
          {labels[state]}
        </Badge>
      }
    >
      <div className="flex flex-wrap items-center gap-3">
        <Button
          onPress={start}
          isDisabled={
            busy ||
            !connected ||
            runtime.synchronization === 'syncing' ||
            state === 'running' ||
            state === 'preparing'
          }
        >
          <Icon name="play" />
          部署并启动应用
        </Button>
        <Button
          variant="outline"
          onPress={stop}
          isDisabled={busy || !connected || state === 'stopped' || state === 'unknown'}
        >
          停止应用
        </Button>
        {app.kind === 'service' && app.healthPath && (
          <Button
            variant="outline"
            onPress={open}
            isDisabled={busy || !runtime.applicationUrls[app.id]}
          >
            打开应用
            <Icon name="browser" />
          </Button>
        )}
      </div>
      <p className="mt-3 text-xs text-muted">
        {connected
          ? '停止应用会保留实例。文件保存与停止计费需要分别确认。'
          : '先启动实例；开机时会自动连接守护进程。已有实例断线时，可在工作台恢复连接。'}
      </p>
    </Panel>
  );
}
export function ApplicationDetail({
  app,
  edit,
  remove,
  controls,
}: {
  app: Recipe;
  edit: (app: Recipe) => void;
  remove: (app: Recipe) => void;
  controls?: ReactNode;
}) {
  return (
    <div className="cd-stack">
      <div className="flex items-center gap-4">
        <div className="cd-app-icon">
          <Icon name={applicationIcon(app)} />
        </div>
        <div className="min-w-0 flex-1">
          <h2 className="text-xl font-semibold">{app.name}</h2>
          <p className="mt-1 text-sm text-muted">{app.description}</p>
        </div>
        <Badge tone={app.builtin ? 'accent' : 'neutral'}>
          {app.builtin ? '项目内置配方' : '自定义配置'}
        </Badge>
      </div>
      {controls}
      <ApplicationDescription text={app.manifest?.details} />
      <Panel title="运行与保存">
        <dl className="cd-properties">
          {app.prepareCommand && (
            <div>
              <dt>准备命令</dt>
              <dd>
                <code>{app.prepareCommand}</code>
              </dd>
            </div>
          )}
          <div>
            <dt>启动命令</dt>
            <dd>
              <code>{app.command}</code>
            </dd>
          </div>
          <div>
            <dt>工作目录</dt>
            <dd>
              <code>{app.workingDirectory}</code>
            </dd>
          </div>
          {app.kind === 'service' && (
            <>
              <div>
                <dt>访问端口</dt>
                <dd>
                  {app.localPort
                    ? `本机 ${app.localPort} → 远端 ${app.remotePort}`
                    : app.remotePort
                      ? `远端 ${app.remotePort}`
                      : '不公开网络端口'}
                </dd>
              </div>
              <div>
                <dt>启动就绪检查</dt>
                <dd>
                  <code>{readinessDescription(app.readiness)}</code>
                  <span className="block text-xs text-muted">
                    等待上限 {app.readiness?.timeoutSeconds ?? 120} 秒；每{' '}
                    {app.readiness?.intervalSeconds ?? 2} 秒检查
                  </span>
                </dd>
              </div>
            </>
          )}
          <div>
            <dt>保存目录</dt>
            <dd>
              {app.saveDirectories.length
                ? app.saveDirectories.map((entry) => (
                    <code key={entry}>
                      {entry}
                      <br />
                    </code>
                  ))
                : '未配置保存目录'}
            </dd>
          </div>
          <div>
            <dt>云盘目标</dt>
            <dd>
              <code>{app.storageDirectory}</code>
            </dd>
          </div>
          <div>
            <dt>启动方式</dt>
            <dd>实例就绪后，按需部署和启动</dd>
          </div>
        </dl>
      </Panel>
      {app.manifest && <ManifestLifecycle app={app} />}
      {app.manifest && app.builtin && (
        <div>
          <Button variant="outline" onPress={() => edit(app)}>
            查看 manifest.json
          </Button>
        </div>
      )}
      {!app.builtin && (
        <div className="flex gap-3">
          <Button onPress={() => edit(app)}>编辑配置</Button>
          <Button variant="danger-soft" onPress={() => remove(app)}>
            移除本地配置
          </Button>
        </div>
      )}
    </div>
  );
}
