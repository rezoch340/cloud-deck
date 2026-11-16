import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import type { Snapshot } from '../domain.ts';
import { cloudLabels } from '../domain.ts';
import { Button } from './ui.tsx';
import { Icon } from './Icon.tsx';

export type Page = 'home' | 'apps' | 'files' | 'activity' | 'settings' | 'detail';
const pages: Record<Page, [string, string]> = {
  home: ['云端工作台', '账号、实例与应用，各自清楚，统一管理。'],
  apps: ['应用', '保存运行方式，用同一台云端机器做更多事。'],
  files: ['文件与同步', '区分下载配置、实例文件和已经保存的文件。'],
  activity: ['活动记录', '每次操作和异常都有记录，刷新不会清除错误。'],
  settings: ['设置', '账号、数据保存和客户端行为。'],
  detail: ['应用配置', '查看运行方式、固定版本和保存规则。'],
};
export function WorkspaceShell({
  page,
  navigate,
  snapshot,
  title,
  actions,
  alerts,
  children,
  toast,
  welcome,
}: {
  page: Page;
  navigate: (page: Page) => void;
  snapshot: Snapshot | null;
  title?: string;
  actions?: ReactNode;
  alerts?: ReactNode;
  children: ReactNode;
  toast?: string;
  welcome: boolean;
}) {
  const [platform, setPlatform] = useState(navigator.userAgent.includes('Mac') ? 'mac' : 'win');
  const heading = useRef<HTMLHeadingElement>(null);
  const main = useRef<HTMLDivElement>(null);
  // 将当前平台标识挂到根文档节点，保证弹窗与下拉浮层也能匹配平台专属滚动条样式。
  useEffect(() => {
    document.documentElement.dataset.platform = platform;
  }, [platform]);
  useEffect(() => {
    heading.current?.focus({ preventScroll: true });
    main.current?.scrollTo(0, 0);
  }, [page]);
  const preview = snapshot?.mode !== 'desktop';
  const cloud = snapshot ? cloudLabels(snapshot.observation) : null;
  const currentVersion = snapshot?.version || '0.1.0';
  const hasUpdate = snapshot?.update?.status === 'available';
  const states = Object.values(snapshot?.runtime.applications || {});
  const appState = states.includes('failed')
    ? '有失败项'
    : states.includes('running')
      ? '运行中'
      : states.includes('preparing')
        ? '准备中'
        : states.length && states.every((value) => value === 'stopped')
          ? '已停止'
          : '未查询';
  const connection = { not_connected: '未建立', connected: '已连接', disconnected: '已中断' }[
    snapshot?.runtime.connection || 'not_connected'
  ];
  const sync = {
    unobserved: '本次未查询',
    pending: '待同步',
    syncing: '同步中',
    verified: '已校验',
    failed: '失败',
  }[snapshot?.runtime.synchronization || 'unobserved'];
  return (
    <div className={`cd-root ${preview ? 'cd-preview' : 'cd-native'}`} data-platform={platform}>
      {preview && (
        <div className="cd-preview-bar">
          <span>
            <b>Cloud Deck</b>
            <span className="cd-preview-description">
              浏览器预览 · 配置仅存本浏览器 · 不访问云端
            </span>
          </span>
          <div className="flex gap-1">
            <Button
              size="sm"
              variant={platform === 'win' ? 'secondary' : 'ghost'}
              aria-pressed={platform === 'win'}
              onPress={() => setPlatform('win')}
            >
              Windows
            </Button>
            <Button
              size="sm"
              variant={platform === 'mac' ? 'secondary' : 'ghost'}
              aria-pressed={platform === 'mac'}
              onPress={() => setPlatform('mac')}
            >
              macOS
            </Button>
          </div>
        </div>
      )}
      <div className="cd-window">
        {preview && (
          <div className="cd-titlebar">
            <span className="cd-traffic" aria-hidden="true">
              <i />
              <i />
              <i />
            </span>
            <span className="cd-window-caption">
              <Icon name="deck" />
              Cloud Deck
            </span>
            <span className="cd-window-buttons" aria-hidden="true">
              —　□　×
            </span>
          </div>
        )}
        <aside className="cd-sidebar">
          <div className="cd-brand">
            <div className="cd-logo">
              <Icon name="deck" />
            </div>
            <div>
              <b>Cloud Deck</b>
              <p>你的云端工作台</p>
            </div>
          </div>
          <nav aria-label="主要导航">
            {(['home', 'apps', 'files', 'activity'] as Page[]).map((item) => (
              <Button
                key={item}
                fullWidth
                variant="ghost"
                className="cd-nav-button"
                data-current={page === item || (page === 'detail' && item === 'apps')}
                aria-current={
                  page === item || (page === 'detail' && item === 'apps') ? 'page' : undefined
                }
                onPress={() => navigate(item)}
                aria-label={pages[item][0]}
              >
                <Icon name={item === 'files' ? 'folder' : item} />
                <span>{pages[item][0]}</span>
                {item === 'apps' && snapshot && <small>{snapshot.apps.length}</small>}
              </Button>
            ))}
          </nav>
          <div className="cd-sidebar-bottom">
            <div className="cd-mini-state">
              <Icon name="cloud" />
              <div>
                <b>{snapshot?.account ? cloud?.instance : '尚未连接账号'}</b>
                <p>{snapshot?.account ? cloud?.billing : '登录不会申请实例'}</p>
              </div>
            </div>
            <Button
              fullWidth
              variant="ghost"
              className="cd-nav-button"
              data-current={page === 'settings'}
              onPress={() => navigate('settings')}
              aria-label="设置"
            >
              <Icon name="settings" />
              <span>设置</span>
              {hasUpdate && <small className="cd-update-dot">新版本</small>}
            </Button>
            <Button
              variant="ghost"
              className="cd-account-button"
              fullWidth
              onPress={() => navigate('settings')}
              aria-label="账号设置"
            >
              <span className="cd-avatar">
                {snapshot?.account?.name.slice(0, 1) || <Icon name="user" />}
              </span>
              <span className="cd-account-text">
                <strong>{snapshot?.account?.name || '连接 Google 账号'}</strong>
                <small>{snapshot?.account?.email || '在浏览器登录'}</small>
              </span>
              <Icon name="chevron" />
            </Button>
          </div>
        </aside>
        <main className={welcome ? 'cd-main cd-main-welcome' : 'cd-main'}>
          {!welcome && (
            <header className="cd-page-header">
              <div>
                {page === 'detail' && (
                  <Button
                    size="sm"
                    variant="ghost"
                    className="-ml-3 mb-1"
                    onPress={() => navigate('apps')}
                  >
                    <Icon name="back" />
                    返回应用
                  </Button>
                )}
                <h1 ref={heading} tabIndex={-1}>
                  {title || pages[page][0]}
                </h1>
                <p>{pages[page][1]}</p>
              </div>
              {actions}
            </header>
          )}
          {alerts && <div className="cd-alerts">{alerts}</div>}
          <div ref={main} className="cd-page-content">
            {children}
          </div>
        </main>
        <footer className="cd-statusbar">
          <div>
            <span>
              <Icon name="cloud" />
              实例：{snapshot?.account ? cloud?.instance : '未查询'}
            </span>
            <span>
              <Icon name="apps" />
              应用：{appState}
            </span>
            <span>
              <Icon name="link" />
              连接：{connection}
            </span>
            <span>
              <Icon name="folder" />
              同步：{sync}
            </span>
          </div>
          <span>
            {snapshot?.account ? cloud?.billing : '登录不会申请实例'}
            {hasUpdate && snapshot?.update?.latestVersion && (
              <button type="button" className="cd-update-pill" onClick={() => navigate('settings')}>
                可更新 v{snapshot.update.latestVersion}
              </button>
            )}
            <span className="cd-version">v{currentVersion}</span>
          </span>
        </footer>
        {toast && (
          <div className="cd-toast" role="status">
            {toast}
          </div>
        )}
      </div>
    </div>
  );
}
