import type { ReactNode } from 'react';
import { Badge, Button, EmptyState, Notice, Panel } from '../components/ui.tsx';
import { GoogleMark, Icon } from '../components/Icon.tsx';
import type { AuthProgress, Snapshot } from '../domain.ts';
import { cloudLabels, mayLeaveAccount, observedTime } from '../domain.ts';

export function LoginButton({
  label = '使用 Google 账号继续',
  disabled,
  press,
}: {
  label?: string;
  disabled: boolean;
  press: () => void;
}) {
  return (
    <Button variant="outline" isDisabled={disabled} onPress={press}>
      <GoogleMark />
      {label}
    </Button>
  );
}
export function Welcome({
  auth,
  login,
  browse,
  reopen,
  cancel,
}: {
  auth: AuthProgress | null;
  login: ReactNode;
  browse: () => void;
  reopen: () => void;
  cancel: () => void;
}) {
  return (
    <div className="cd-welcome">
      <div className="cd-welcome-grid">
        <section>
          <h1>
            {auth ? (
              auth.phase === 'checking' ? (
                '账号已登录，正在检查 Colab'
              ) : auth.phase === 'verifying' ? (
                '正在验证 Google 授权'
              ) : (
                '请在浏览器完成登录'
              )
            ) : (
              <>
                使用 Google 登录，
                <br />
                直接打开你的应用。
              </>
            )}
          </h1>
          <p className="cd-intro">
            {auth ? auth.message : '连接你已有的 Colab。环境、连接和文件保存，由同一个工作台管理。'}
          </p>
          {auth ? (
            <>
              <div className="cd-waiting" role="status">
                <span className="cd-spinner" />
                {auth.message}
              </div>
              <div className="cd-actions">
                <Button variant="outline" onPress={reopen} isDisabled={auth.phase !== 'waiting'}>
                  <Icon name="browser" />
                  重新打开浏览器
                </Button>
                <Button variant="ghost" isDisabled={auth.phase === 'checking'} onPress={cancel}>
                  取消登录
                </Button>
              </div>
            </>
          ) : (
            <>
              <div className="cd-actions">{login}</div>
              <p className="mt-3 text-xs text-muted">
                在系统浏览器选择账号，授权后自动返回客户端。
              </p>
              <Button variant="ghost" size="sm" className="mt-4 -ml-3" onPress={browse}>
                先看看内置应用
                <Icon name="chevron" />
              </Button>
            </>
          )}
        </section>
        <div
          className="cd-handoff"
          role="img"
          aria-label="流程示意：Google 浏览器授权后自动回到客户端"
        >
          <Panel
            className="cd-browser-art"
            title={
              <span className="flex items-center gap-2">
                <GoogleMark />
                在浏览器选择账号
              </span>
            }
            description="使用你自己的 Google 账号"
          >
            <div className="cd-illustration-account">
              <span className="cd-avatar">
                <Icon name="user" />
              </span>
              <span>你的 Google 账号</span>
              <Icon name="chevron" />
            </div>
          </Panel>
          <div className="cd-transfer" />
          <Panel
            className="cd-client-art"
            title={
              <span className="flex items-center gap-2">
                <Icon name="deck" />
                Cloud Deck
              </span>
            }
          >
            <div className="flex items-center gap-3">
              <span className="cd-check-circle">
                <Icon name="check" />
              </span>
              <div>
                <b>授权后自动返回</b>
                <p className="mt-1 text-xs text-muted">继续配置你的应用</p>
              </div>
            </div>
          </Panel>
        </div>
      </div>
      <div className="cd-welcome-features">
        <div>
          <Icon name="user" />
          <h3>账号由你选择</h3>
          <p>
            沿用自己的 Google 账号，
            <br />
            无需另注册账户。
          </p>
        </div>
        <div>
          <Icon name="node" />
          <h3>ComfyUI 已有配置</h3>
          <p>
            使用我们的固定版本，
            <br />
            插件与模型按清单恢复。
          </p>
        </div>
        <div>
          <Icon name="folder" />
          <h3>文件留在自己的云盘</h3>
          <p>
            工作流与结果保存到 Drive，
            <br />
            保存完成后再关机。
          </p>
        </div>
      </div>
    </div>
  );
}
const category: Record<string, string> = {
  account: '账号',
  application: '应用',
  instance: '实例',
  connection: '连接',
};
export function ActivityView({ snapshot }: { snapshot: Snapshot }) {
  if (!snapshot.events.length)
    return (
      <EmptyState title="还没有活动记录">登录、状态查询和应用配置变更会记录在这里。</EmptyState>
    );
  return (
    <Panel>
      <div className="cd-log-list">
        {[...snapshot.events].reverse().map((event, index) => (
          <article className="cd-log-row" key={`${event.time}-${index}`}>
            <time>{observedTime(event.time)}</time>
            <Badge tone={event.level === 'error' ? 'bad' : 'neutral'}>
              {category[event.category] || event.category}
            </Badge>
            <p>{event.message}</p>
          </article>
        ))}
      </div>
    </Panel>
  );
}
export function Settings({
  snapshot,
  login,
  signOut,
  busy,
}: {
  snapshot: Snapshot;
  login: ReactNode;
  signOut: () => void;
  busy: boolean;
}) {
  return (
    <div className="cd-stack">
      <Panel title="Google 账号">
        {snapshot.account ? (
          <>
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div>
                <strong>{snapshot.account.name}</strong>
                <p className="mt-1 text-sm text-muted">{snapshot.account.email}</p>
              </div>
              <Badge tone={snapshot.account.status === 'connected' ? 'good' : 'neutral'}>
                {snapshot.account.status === 'connected' ? '身份已验证' : '待重新查询'}
              </Badge>
            </div>
            <div className="cd-actions">
              {login}
              <Button
                variant="tertiary"
                isDisabled={busy || !mayLeaveAccount(snapshot)}
                onPress={signOut}
              >
                退出本机账号
              </Button>
            </div>
            {!mayLeaveAccount(snapshot) && (
              <div className="mt-4">
                <Notice tone="warn">
                  实例或计费尚未确认，不能直接退出或切换账号。请先刷新状态；重新登录时必须使用原账号。
                </Notice>
              </div>
            )}
          </>
        ) : (
          <>
            <p className="text-sm text-muted">在系统浏览器选择账号，令牌保存在系统凭据存储。</p>
            <div className="cd-actions">{login}</div>
          </>
        )}
      </Panel>
      <Panel title="本地数据">
        <dl className="cd-properties">
          <div>
            <dt>应用配置与记录</dt>
            <dd>
              <code>{snapshot.dataDirectory}</code>
            </dd>
          </div>
          <div>
            <dt>授权资料</dt>
            <dd>
              {snapshot.mode === 'desktop'
                ? 'macOS 钥匙串 / Windows 凭据管理器'
                : '预览不保存授权，也不读取项目 auth 目录'}
            </dd>
          </div>
          <div>
            <dt>内置配置来源</dt>
            <dd>项目 ComfyUI 配方、manifest.json 与云端启动脚本</dd>
          </div>
        </dl>
      </Panel>
      <Panel title="云端运行方式">
        <p className="text-sm leading-7 text-muted">
          客户端管理连接，云端守护进程管理应用。关闭窗口不会停止机器或计费；停止应用与释放实例分别处理。
        </p>
        <div className="mt-4">
          <Notice>
            此版本可连接实例并管理应用。保存关机先停止应用、同步并核对 Drive
            服务端校验值，全部通过后才释放实例并确认停止计费。失败保留实例及本地文件。
          </Notice>
        </div>
      </Panel>
    </div>
  );
}
