import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { Modal } from '@heroui/react';
import * as api from './api.ts';
import { mayLeaveAccount, newDraft } from './domain.ts';
import type { AppKind, AuthProgress, Recipe, RecipeDraft, Snapshot } from './domain.ts';
import { Button, Dialog, EmptyState, Notice } from './components/ui.tsx';
import { Icon } from './components/Icon.tsx';
import { ApplicationLogs } from './components/ApplicationLogs.tsx';
import { ApplicationEditor } from './components/ApplicationEditor.tsx';
import { WorkspaceShell } from './components/WorkspaceShell.tsx';
import type { Page } from './components/WorkspaceShell.tsx';
import { ApplicationDetail, ApplicationRuntime, Applications } from './pages/ApplicationViews.tsx';
import { ActivityView, LoginButton, Settings, Welcome } from './pages/WorkspaceViews.tsx';
import { ManifestEditor } from './components/ManifestEditor.tsx';
import { newApplicationManifest } from './manifest.ts';
import type { ApplicationManifest } from './manifest.ts';
import { StartInstanceDialog, Workspace } from './pages/Workspace.tsx';
import { Files } from './pages/Files.tsx';
import './styles.css';

export default function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState('');
  const [page, setPage] = useState<Page>('home');
  const [selected, setSelected] = useState('comfyui');
  const [manifestEditor, setManifestEditor] = useState<{
    manifest: ApplicationManifest;
    readOnly: boolean;
  } | null>(null);
  const [draft, setDraft] = useState<RecipeDraft | null>(null);
  const [remove, setRemove] = useState<Recipe | null>(null);
  const [auth, setAuth] = useState<AuthProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState('');
  const [startingInstance, setStartingInstance] = useState(false);
  const [release, setRelease] = useState(false);
  const [saving, setSaving] = useState<'save' | 'shutdown' | null>(null);

  async function load() {
    try {
      setSnapshot(await api.getWorkspace());
      setError('');
    } catch (error) {
      setError(String(error instanceof Error ? error.message : error));
    }
  }
  useEffect(() => {
    void load();
    let stop: (() => void) | undefined;
    let disposed = false;
    api
      .onAuthProgress(setAuth)
      .then((unsubscribe) => {
        if (disposed) unsubscribe();
        else stop = unsubscribe;
      })
      .catch(() => {
        if (!disposed) setError('无法订阅登录进度，请重新打开客户端。');
      });
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);
  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(''), 4000);
    return () => clearTimeout(timer);
  }, [toast]);
  useEffect(() => {
    if (!api.desktop) return;
    let active = true;
    let polling = false;
    const timer = setInterval(() => {
      if (polling) return;
      polling = true;
      void (busy || !snapshot?.runtime.endpoint ? api.getWorkspace() : api.refreshCloud())
        .then((next) => {
          if (active) setSnapshot(next);
        })
        .catch((error) => {
          if (active) setError(String(error));
        })
        .finally(() => {
          polling = false;
        });
    }, 2500);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [busy, snapshot?.account?.subject, snapshot?.runtime.endpoint]);

  async function perform(operation: () => Promise<Snapshot>) {
    setBusy(true);
    setError('');
    try {
      setSnapshot(await operation());
    } catch (error) {
      setError(String(error instanceof Error ? error.message : error));
      const current = await api.getWorkspace().catch(() => null);
      if (current) setSnapshot(current);
    } finally {
      setBusy(false);
      setAuth(null);
    }
  }
  function navigate(next: Page) {
    setPage(next);
    setError('');
  }
  function open(app: Recipe) {
    setSelected(app.id);
    navigate('detail');
  }
  function add(kind: AppKind = 'service') {
    setDraft(newDraft(kind));
  }
  function edit(app: Recipe) {
    if (app.manifest) {
      setManifestEditor({ manifest: app.manifest, readOnly: app.builtin });
      return;
    }
    const { builtin: builtinConfiguration, manifest: manifestMetadata, ...value } = app;
    setDraft(value);
  }
  function login() {
    setAuth({ phase: 'preparing', message: '正在准备 Google 登录…' });
    void perform(api.signIn);
  }
  function notifyFailure(error: unknown) {
    setError(String(error instanceof Error ? error.message : error));
  }
  const current = snapshot?.apps.find((app) => app.id === selected) || snapshot?.apps[0];
  const welcome = page === 'home' && !snapshot?.account;
  const loginControl = (
    <LoginButton
      press={login}
      disabled={!api.desktop || busy}
      label={
        snapshot?.account ? (mayLeaveAccount(snapshot) ? '切换账号' : '重新连接原账号') : undefined
      }
    />
  );
  const addControl = (
    <Button variant="primary" onPress={() => add()}>
      <Icon name="plus" />
      添加应用
    </Button>
  );

  function content(): ReactNode {
    if (!snapshot)
      return (
        <EmptyState
          title={error ? '工作空间未能加载' : '正在打开工作空间…'}
          action={error && <Button onPress={() => void load()}>重新读取</Button>}
        >
          {error}
        </EmptyState>
      );
    if (welcome)
      return (
        <Welcome
          auth={auth}
          login={loginControl}
          browse={() => navigate('apps')}
          reopen={() => void api.reopenSignIn().catch(notifyFailure)}
          cancel={() => void api.cancelSignIn().catch(notifyFailure)}
        />
      );
    switch (page) {
      case 'home':
        return (
          <Workspace
            snapshot={snapshot}
            open={open}
            apps={() => navigate('apps')}
            settings={() => navigate('settings')}
            activity={() => navigate('activity')}
            busy={busy}
            start={() => setStartingInstance(true)}
            reconnect={(endpoint) => void perform(() => api.connectInstance(endpoint))}
            refresh={() => void perform(api.refreshAccount)}
            release={() => setRelease(true)}
            shutdown={() => setSaving('shutdown')}
          />
        );
      case 'apps':
        return (
          <Applications
            apps={snapshot.apps}
            open={open}
            add={add}
            addManifest={() =>
              setManifestEditor({ manifest: newApplicationManifest(), readOnly: false })
            }
          />
        );
      case 'detail':
        return (
          current && (
            <ApplicationDetail
              app={current}
              edit={edit}
              remove={setRemove}
              controls={
                <>
                  <ApplicationRuntime
                    app={current}
                    runtime={snapshot.runtime}
                    busy={busy}
                    start={() => void perform(() => api.startApplication(current.id))}
                    stop={() => void perform(() => api.stopApplication(current.id))}
                    open={() => void api.openApplication(current.id).catch(notifyFailure)}
                  />
                  <ApplicationLogs
                    key={`${snapshot.runtime.endpoint}:${current.id}`}
                    app={current}
                    runtime={snapshot.runtime}
                  />
                </>
              }
            />
          )
        );
      case 'files':
        return (
          current && (
            <Files
              apps={snapshot.apps}
              app={current}
              select={setSelected}
              runtime={snapshot.runtime}
              busy={busy}
              save={() => setSaving('save')}
            />
          )
        );
      case 'activity':
        return <ActivityView snapshot={snapshot} />;
      case 'settings':
        return (
          <Settings
            snapshot={snapshot}
            login={loginControl}
            busy={busy}
            signOut={() => void perform(api.signOut)}
          />
        );
    }
  }

  const alerts = (
    <>
      {error && snapshot && (
        <Notice title="操作未完成" tone="bad">
          {error}
        </Notice>
      )}
      {snapshot?.observation.error && !welcome && page === 'home' && (
        <Notice title="查询未完成，云端状态待确认" tone="warn">
          {snapshot.observation.error} 上次记录仍保留。
        </Notice>
      )}
      {auth && !welcome && (
        <Notice title="Google 授权正在进行" tone="accent">
          {auth.message}
          {auth.phase === 'waiting' && (
            <Button
              variant="ghost"
              size="sm"
              onPress={() => void api.cancelSignIn().catch(notifyFailure)}
            >
              取消登录
            </Button>
          )}
        </Notice>
      )}
      {busy && snapshot?.runtime.phase !== 'idle' && !auth && (
        <Notice title="云端操作正在进行" tone="accent">
          {snapshot?.runtime.message}
        </Notice>
      )}
      {snapshot?.runtime.needsDriveConsent && (
        <Notice title="需要允许 Google Drive 访问" tone="warn">
          请在浏览器完成授权，客户端会自动继续挂载和启动应用，无需重复点击启动。等待期间实例仍可能计费。
          <Button
            variant="outline"
            size="sm"
            onPress={() => void api.openDriveConsent().catch(notifyFailure)}
          >
            打开 Drive 授权
          </Button>
        </Notice>
      )}
    </>
  );
  return (
    <>
      <WorkspaceShell
        page={page}
        navigate={navigate}
        snapshot={snapshot}
        welcome={welcome}
        title={page === 'detail' ? current?.name : undefined}
        toast={toast}
        alerts={
          error ||
          snapshot?.observation.error ||
          auth ||
          busy ||
          snapshot?.runtime.needsDriveConsent
            ? alerts
            : undefined
        }
        actions={
          page === 'apps' ? (
            addControl
          ) : page === 'home' || page === 'settings' ? (
            <Button
              variant="outline"
              isDisabled={!api.desktop || !snapshot?.account || busy}
              isPending={busy}
              onPress={() => void perform(api.refreshAccount)}
            >
              <Icon name="refresh" />
              {busy ? '正在处理…' : '刷新状态'}
            </Button>
          ) : undefined
        }
      >
        {content()}
      </WorkspaceShell>
      {startingInstance && snapshot && (
        <StartInstanceDialog
          snapshot={snapshot}
          close={() => setStartingInstance(false)}
          confirm={(accelerator) => {
            setStartingInstance(false);
            void perform(() => api.startInstance(accelerator));
          }}
        />
      )}
      {release && (
        <Dialog title="释放本次新建的空实例？" close={() => setRelease(false)}>
          <Modal.Body>
            <p className="text-sm leading-7 text-muted">
              仅释放本次新建、从未运行过应用的空实例。已经运行应用或保存状态未确认时，会保留实例并阻止释放。
            </p>
          </Modal.Body>
          <Modal.Footer>
            <Button variant="tertiary" onPress={() => setRelease(false)}>
              取消
            </Button>
            <Button
              variant="danger"
              onPress={() => {
                setRelease(false);
                void perform(api.releaseEmptyInstance);
              }}
            >
              确认释放空实例
            </Button>
          </Modal.Footer>
        </Dialog>
      )}
      {saving && (
        <Dialog
          title={saving === 'shutdown' ? '保存并关闭实例？' : '停止应用并保存文件？'}
          close={() => setSaving(null)}
        >
          <Modal.Body>
            <p className="text-sm leading-7 text-muted">
              先停止此实例的受管应用，将配置的保存目录复制到 Drive，再核对 Drive
              服务端文件大小和校验值。
              {saving === 'shutdown'
                ? '全部通过后才释放实例，并重新查询计费。'
                : '完成后保留实例，可以重新启动应用。'}
              任何步骤失败都会保留实例及本地文件。
            </p>
          </Modal.Body>
          <Modal.Footer>
            <Button variant="tertiary" onPress={() => setSaving(null)}>
              取消
            </Button>
            <Button
              variant={saving === 'shutdown' ? 'danger' : 'primary'}
              onPress={() => {
                const operation = saving === 'shutdown' ? api.shutdownCloud : api.synchronizeFiles;
                setSaving(null);
                void perform(operation);
              }}
            >
              {saving === 'shutdown' ? '保存并关机' : '保存并校验'}
            </Button>
          </Modal.Footer>
        </Dialog>
      )}
      {manifestEditor && (
        <ManifestEditor
          initial={manifestEditor.manifest}
          readOnly={manifestEditor.readOnly}
          close={() => setManifestEditor(null)}
          save={async (manifest) => {
            setSnapshot(await api.saveApplicationManifest(manifest));
            setSelected(manifest.id);
            setPage('detail');
            setToast('应用清单已保存。');
          }}
        />
      )}
      {draft && snapshot && (
        <ApplicationEditor
          initial={draft}
          apps={snapshot.apps}
          close={() => setDraft(null)}
          save={async (value) => {
            const next = await api.saveApplication(value);
            setSnapshot(next);
            setSelected(value.id || next.apps.at(-1)!.id);
            setPage('apps');
            setToast('应用配置已保存。没有启动云端程序。');
          }}
        />
      )}
      {remove && (
        <Dialog title={`移除 ${remove.name} 的本地配置？`} close={() => setRemove(null)}>
          <Modal.Body>
            <p className="text-sm leading-7 text-muted">
              只移除当前客户端的应用配置，不停止云端应用，也不删除任何云端文件。
            </p>
          </Modal.Body>
          <Modal.Footer>
            <Button variant="tertiary" onPress={() => setRemove(null)}>
              取消
            </Button>
            <Button
              variant="danger"
              onPress={() => {
                void api
                  .removeApplication(remove.id)
                  .then((next) => {
                    setSnapshot(next);
                    navigate('apps');
                    setRemove(null);
                    setToast('本地配置已移除，云端文件未改动。');
                  })
                  .catch(notifyFailure);
              }}
            >
              移除配置
            </Button>
          </Modal.Footer>
        </Dialog>
      )}
    </>
  );
}
