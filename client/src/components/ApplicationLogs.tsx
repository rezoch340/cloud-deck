import { useEffect, useRef, useState } from 'react';
import { applicationLogs } from '../api.ts';
import type { ApplicationLog, Recipe, Snapshot } from '../domain.ts';
import { Button, Notice, Panel } from './ui.tsx';

export function ApplicationLogs({ app, runtime }: { app: Recipe; runtime: Snapshot['runtime'] }) {
  const [result, setResult] = useState<ApplicationLog | null>(null);
  const [error, setError] = useState('');
  const [following, setFollowing] = useState(true);
  const output = useRef<HTMLPreElement>(null);
  const connected = runtime.connection === 'connected';

  useEffect(() => {
    if (!connected) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    async function refresh() {
      try {
        const next = await applicationLogs(app.id);
        if (active) {
          setResult(next);
          setError('');
        }
      } catch (failure) {
        if (active) setError(String(failure));
      } finally {
        if (active) timer = setTimeout(() => void refresh(), 2000);
      }
    }
    void refresh();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [app.id, connected]);

  useEffect(() => {
    if (following && output.current) output.current.scrollTop = output.current.scrollHeight;
  }, [following, result?.text]);

  const state = result?.state || runtime.applications[app.id];
  const submitting = ['mounting_drive', 'drive_consent', 'starting'].includes(runtime.phase);
  const phase = submitting
    ? runtime.message
    : state === 'failed'
      ? (result?.progress?.stage === 'failed' && result.progress.message) ||
        result?.error ||
        '应用执行失败，请查看下面的错误输出。'
      : state === 'running'
        ? '应用正在运行'
        : state === 'stopped'
          ? '应用已停止'
          : result?.stage === 'prepare'
            ? result.progress?.message || '正在准备环境'
            : state === 'preparing'
              ? '进程已启动，正在等待就绪检测通过'
              : runtime.message;
  const progress = result?.stage === 'prepare' && state === 'preparing' ? result.progress : null;
  const text = (result?.text || '').replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '').replace(/\r/g, '\n');

  return (
    <Panel
      title="部署与运行日志"
      className="cd-application-logs"
      description="资源、准备命令、启动输出与就绪检查，自动更新。"
      actions={
        <Button variant="ghost" onPress={() => setFollowing(!following)} aria-pressed={following}>
          {following ? '暂停跟随' : '跟随最新'}
        </Button>
      }
    >
      <p role="status" className="break-words text-sm font-medium">
        {phase}
      </p>
      {progress?.totalBytes != null && progress.totalBytes > 0 && (
        <div className="mt-3">
          <progress
            aria-label="资源下载进度"
            className="w-full accent-[var(--accent)]"
            max={progress.totalBytes}
            value={progress.completedBytes || 0}
          />
          <p className="text-xs text-muted">
            {(((progress.completedBytes || 0) / progress.totalBytes) * 100).toFixed(1)}%
          </p>
        </div>
      )}
      {error && <Notice tone="warn">读取日志失败，保留上次内容：{error}</Notice>}
      <p className="my-3 text-xs text-muted">
        {connected
          ? '每 2 秒更新'
          : runtime.connection === 'disconnected'
            ? '连接已断开，显示上次内容；恢复连接后继续读取云端日志。'
            : '尚未连接实例，连接后自动读取云端日志。'}
        {result?.observedAt && ` · 最近读取 ${new Date(result.observedAt).toLocaleTimeString()}`}
        {result?.truncated && ' · 仅展示最近 64 KiB'}
      </p>
      <pre
        ref={output}
        tabIndex={0}
        aria-label={`${app.name} 部署与运行日志`}
        className="max-h-80 min-h-32 overflow-auto rounded-xl bg-default p-4 font-mono text-xs leading-6 whitespace-pre-wrap break-all"
      >
        {text || (connected ? '等待部署输出…' : '连接实例后可查看此应用的云端日志。')}
      </pre>
    </Panel>
  );
}
