import { useState } from 'react';
import { Modal } from '@heroui/react';
import { Badge, Button, Dialog, Notice, Panel, SelectInput } from '../components/ui.tsx';
import { Icon } from '../components/Icon.tsx';
import type { Recipe, Snapshot } from '../domain.ts';
import { cloudLabels, instanceAction, instanceRegion, observedTime } from '../domain.ts';
import { ApplicationCard } from './ApplicationViews.tsx';

export function StartInstanceDialog({
  snapshot,
  close,
  confirm,
}: {
  snapshot: Snapshot;
  close: () => void;
  confirm: (accelerator: string) => void;
}) {
  const gpus = snapshot.observation.eligibleGpus;
  const [accelerator, setAccelerator] = useState(gpus[0] || 'NONE');
  return (
    <Dialog title="启动云端实例" close={close}>
      <Modal.Body>
        <SelectInput
          label="计算类型"
          value={accelerator}
          onChange={setAccelerator}
          options={[
            ...gpus.map((value) => ({ value, label: value })),
            { value: 'NONE', label: 'CPU · 不请求 GPU' },
          ]}
        />
        <p className="mt-4 text-sm leading-7 text-muted">
          开机时自动部署并连接云端守护进程。就绪后，再选择需要部署的应用。
        </p>
        <div className="mt-4">
          <Notice tone="warn">
            分配成功后即可能计费，尚未就绪也可能消耗点数。关闭客户端不会释放实例；保存尚未核验时会阻止释放。
          </Notice>
        </div>
      </Modal.Body>
      <Modal.Footer>
        <Button variant="tertiary" onPress={close}>
          取消
        </Button>
        <Button variant="primary" onPress={() => confirm(accelerator)}>
          确认并启动
        </Button>
      </Modal.Footer>
    </Dialog>
  );
}
export function Workspace({
  snapshot,
  open,
  apps,
  settings,
  activity,
  start,
  reconnect,
  refresh,
  release,
  shutdown,
  busy,
}: {
  snapshot: Snapshot;
  open: (app: Recipe) => void;
  apps: () => void;
  settings: () => void;
  activity: () => void;
  start: () => void;
  reconnect: (endpoint: string) => void;
  refresh: () => void;
  release: () => void;
  shutdown: () => void;
  busy: boolean;
}) {
  const cloud = cloudLabels(snapshot.observation);
  const primaryAction = instanceAction(snapshot);
  function runPrimaryAction() {
    switch (primaryAction.kind) {
      case 'start':
        start();
        break;
      case 'connect':
        reconnect(primaryAction.endpoint);
        break;
      case 'refresh':
        refresh();
        break;
      case 'applications':
        apps();
        break;
    }
  }
  const current = snapshot.observation.status === 'current';
  const region = instanceRegion(snapshot.observation);
  return (
    <div className="cd-stack">
      <div className="cd-workspace-grid">
        <div className="cd-stack">
          <Panel
            className="cd-machine"
            title={
              <span className="flex items-center gap-2">
                <Icon name="cloud" />
                Google Colab
              </span>
            }
            actions={<Badge tone={cloud.tone}>{cloud.instance}</Badge>}
          >
            <div className="cd-machine-identity">
              <span className="cd-chip">
                <Icon name="chip" />
              </span>
              <div>
                <h2>
                  {current && snapshot.observation.instances[0]?.accelerator
                    ? `${snapshot.observation.instances[0].accelerator} 云端实例`
                    : '我的云端工作空间'}
                </h2>
                <p>应用独立管理，实例状态来自只读查询。</p>
                <div className="mt-2 flex flex-wrap">
                  <Badge tone={region.historical ? 'neutral' : 'accent'}>
                    {region.label}：{region.value}
                  </Badge>
                </div>
              </div>
            </div>
            <dl className="cd-facts">
              <div>
                <dt>可用计算点数</dt>
                <dd>
                  {current && snapshot.observation.balance !== null
                    ? snapshot.observation.balance.toFixed(2)
                    : '—'}
                  <small> 点</small>
                </dd>
              </div>
              <div>
                <dt>当前消耗</dt>
                <dd>
                  {current && snapshot.observation.hourlyRate !== null
                    ? snapshot.observation.hourlyRate.toFixed(2)
                    : '—'}
                  <small> 点 / 小时</small>
                </dd>
              </div>
              <div>
                <dt>观测状态</dt>
                <dd className="cd-fact-text">
                  {current ? '本次查询' : snapshot.observation.observedAt ? '历史快照' : '尚未查询'}
                </dd>
              </div>
            </dl>
            <div className="cd-machine-action">
              <p>{snapshot.runtime.message}</p>
              <div className="flex flex-wrap gap-2">
                <Button
                  variant="outline"
                  isDisabled={
                    busy || primaryAction.kind === 'unavailable' || snapshot.mode !== 'desktop'
                  }
                  isPending={busy}
                  onPress={runPrimaryAction}
                >
                  <Icon
                    name={
                      primaryAction.kind === 'applications'
                        ? 'node'
                        : primaryAction.kind === 'refresh'
                          ? 'refresh'
                          : 'play'
                    }
                  />
                  {busy ? '正在处理…' : primaryAction.label}
                </Button>
                {snapshot.runtime.endpoint && (
                  <>
                    <Button
                      variant="outline"
                      isDisabled={
                        busy ||
                        snapshot.runtime.connection !== 'connected' ||
                        snapshot.runtime.synchronization === 'syncing'
                      }
                      onPress={shutdown}
                    >
                      保存并关闭实例
                    </Button>
                    <Button
                      variant="ghost"
                      isDisabled={
                        busy ||
                        snapshot.runtime.connection !== 'connected' ||
                        snapshot.runtime.synchronization === 'syncing'
                      }
                      onPress={release}
                    >
                      释放空实例
                    </Button>
                  </>
                )}
              </div>
            </div>
          </Panel>
          <section>
            <div className="cd-section-heading">
              <h2>
                我的应用 <span>{snapshot.apps.length}</span>
              </h2>
              <Button variant="ghost" size="sm" onPress={apps}>
                管理应用
                <Icon name="chevron" />
              </Button>
            </div>
            <div className="cd-stack-sm">
              {snapshot.apps.map((app) => (
                <ApplicationCard key={app.id} app={app} open={open} />
              ))}
            </div>
          </section>
        </div>
        <aside className="cd-stack">
          <Panel title="当前账号">
            <div className="cd-account-summary">
              <span className="cd-avatar">{snapshot.account?.name.slice(0, 1) || 'G'}</span>
              <div className="min-w-0">
                <strong>{snapshot.account?.name}</strong>
                <small>{snapshot.account?.email}</small>
              </div>
            </div>
            <p className="my-3 text-xs text-muted">
              {snapshot.account?.status === 'connected'
                ? 'Google 身份已验证'
                : '已保存账号，等待重新查询'}
            </p>
            <Button variant="ghost" size="sm" onPress={settings}>
              管理账号
              <Icon name="chevron" />
            </Button>
          </Panel>
          <Panel title="控制与保存">
            <dl className="cd-status-list">
              <div>
                <dt>云端守护进程</dt>
                <dd>
                  {snapshot.runtime.connection === 'connected'
                    ? '已连接'
                    : snapshot.runtime.connection === 'disconnected'
                      ? '连接中断'
                      : '尚未连接'}
                </dd>
              </div>
              <div>
                <dt>云端实例保活</dt>
                <dd>
                  {snapshot.runtime.heartbeat?.status === 'confirmed'
                    ? snapshot.runtime.connection === 'connected'
                      ? '云端 kernel 活动已确认'
                      : '上次云端活动已确认，当前连接中断'
                    : snapshot.runtime.heartbeat?.status === 'error'
                      ? '云端活动未确认，将自动重试'
                      : snapshot.runtime.heartbeat?.status === 'waiting'
                        ? '云端守护正在建立会话活动'
                        : '未启用'}
                  {snapshot.runtime.heartbeat?.owner === 'cloud' && (
                    <span className="block text-xs text-muted">
                      由云端守护执行，客户端关闭后继续。
                    </span>
                  )}
                  {snapshot.runtime.heartbeat?.lastAttemptAt && (
                    <span className="block text-xs text-muted">
                      {observedTime(snapshot.runtime.heartbeat.lastAttemptAt)}
                    </span>
                  )}
                  {snapshot.runtime.heartbeat?.error && (
                    <span className="block text-xs text-muted">
                      {snapshot.runtime.heartbeat.error}
                    </span>
                  )}
                </dd>
              </div>
              <div>
                <dt>应用入口</dt>
                <dd>
                  {Object.keys(snapshot.runtime.applicationUrls).length ? '健康检查通过' : '未建立'}
                </dd>
              </div>
              <div>
                <dt>文件同步</dt>
                <dd>
                  {
                    {
                      unobserved: '未检查',
                      pending: '待同步',
                      syncing: '正在保存与校验',
                      verified:
                        snapshot.runtime.storage.method === 'no_save_rules'
                          ? '没有待保存文件'
                          : 'Drive 服务端已校验',
                      failed: '失败，实例保留',
                    }[snapshot.runtime.synchronization]
                  }
                </dd>
              </div>
            </dl>
            <p className="mt-4 text-xs text-muted">登录成功不代表应用已运行或文件已保存。</p>
          </Panel>
          <Notice tone={cloud.confirmedOff ? 'neutral' : 'warn'}>
            关闭客户端不会释放实例。{cloud.billing}。
          </Notice>
        </aside>
      </div>
      <div className="cd-activity-strip">
        <Icon name="activity" />
        <span>{snapshot.events.at(-1)?.message || '本次还没有操作记录。'}</span>
        <Button variant="ghost" size="sm" onPress={activity}>
          全部记录
        </Button>
      </div>
    </div>
  );
}
