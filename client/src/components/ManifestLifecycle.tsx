import type { Recipe } from '../domain.ts';
import { bytes } from '../domain.ts';
import { Panel, DataTable } from './ui.tsx';

export function ManifestLifecycle({ app }: { app: Recipe }) {
  const manifest = app.manifest;
  if (!manifest) return null;
  return (
    <Panel title="清单生命周期">
      <p className="mb-4 text-sm text-muted">
        有效本地资源直接复用；需要获取时按清单顺序尝试来源。启动恢复只补缺失文件，保留实例上的修改。
      </p>
      {manifest.resources.files.length > 0 && (
        <DataTable
          label="资源与来源优先级"
          columns={['实例资源', '大小', '来源顺序']}
          rows={manifest.resources.files.map((resource) => ({
            id: resource.target,
            cells: [
              <code>{resource.target}</code>,
              bytes(resource.size),
              resource.sources
                .map((source) =>
                  source.kind === 'download'
                    ? `下载 · ${new URL(source.url).hostname}`
                    : 'Google Drive',
                )
                .join(' → '),
            ],
          }))}
        />
      )}
      <dl className="cd-properties mt-4">
        <div>
          <dt>启动恢复</dt>
          <dd>
            {manifest.startup.restore.length
              ? manifest.startup.restore.map((rule) => (
                  <div key={rule.target}>
                    <code>
                      {rule.source} → {rule.target}
                    </code>
                    {rule.optional ? '（可选）' : ''}
                  </div>
                ))
              : '无'}
          </dd>
        </div>
        <div>
          <dt>准备步骤</dt>
          <dd>
            {manifest.startup.commands.length ? (
              <details>
                <summary>{manifest.startup.commands.length} 个步骤，查看命令</summary>
                {manifest.startup.commands.map((step, index) => (
                  <div key={index}>
                    <code>{step.command}</code>
                  </div>
                ))}
              </details>
            ) : (
              '无'
            )}
          </dd>
        </div>
        <div>
          <dt>关机保存</dt>
          <dd>
            {manifest.shutdown.save.length
              ? manifest.shutdown.save.map((rule) => (
                  <div key={rule.source}>
                    <code>
                      {rule.source} → {rule.target}
                    </code>
                  </div>
                ))
              : '无'}
          </dd>
        </div>
      </dl>
    </Panel>
  );
}
