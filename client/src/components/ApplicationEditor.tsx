import { useState } from 'react';
import { Form, Modal } from '@heroui/react';
import type { FormEvent } from 'react';
import type { AppKind, Recipe, RecipeDraft } from '../domain.ts';
import { validateRecipe } from '../domain.ts';
import { Button, Dialog, Notice, PortInput, SelectInput, TextInput } from './ui.tsx';

export function ApplicationEditor({
  initial,
  apps,
  save,
  close,
}: {
  initial: RecipeDraft;
  apps: Recipe[];
  save: (draft: RecipeDraft) => Promise<void>;
  close: () => void;
}) {
  const [draft, setDraft] = useState(initial);
  const [directories, setDirectories] = useState(initial.saveDirectories.join('\n'));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const field = <FieldName extends keyof RecipeDraft>(
    name: FieldName,
    value: RecipeDraft[FieldName],
  ) => setDraft((current) => ({ ...current, [name]: value }));
  function changeKind(kind: AppKind) {
    setDraft((current) => ({
      ...current,
      kind,
      remotePort: kind === 'service' ? current.remotePort || 7860 : null,
      localPort: kind === 'service' ? current.localPort || 17860 : null,
      healthPath: kind === 'service' ? current.healthPath || '/' : null,
    }));
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    const value = {
      ...draft,
      autoStart: false,
      saveDirectories: directories
        .split('\n')
        .map((value) => value.trim())
        .filter(Boolean),
    };
    const invalid = validateRecipe(value, apps);
    if (invalid) {
      setError(invalid);
      return;
    }
    setError('');
    setSaving(true);
    try {
      await save(value);
      close();
    } catch (error) {
      setError(String(error instanceof Error ? error.message : error));
    } finally {
      setSaving(false);
    }
  }
  return (
    <Dialog title={draft.id ? '编辑应用配置' : '添加应用'} close={close} locked={saving}>
      <Form onSubmit={submit} className="cd-modal-form">
        {error && (
          <div role="alert">
            <Notice tone="bad">{error}</Notice>
          </div>
        )}
        <Modal.Body>
          <div className="cd-form-grid">
            <p className="col-span-full text-sm text-muted">
              保存云端运行方式，不会安装或启动程序。
            </p>
            <TextInput
              label="应用名称"
              name="name"
              value={draft.name}
              onChange={(value) => field('name', value)}
              isRequired
              autoFocus
              maxLength={60}
              placeholder="例如：我的图像服务"
            />
            <SelectInput
              label="应用类型"
              name="kind"
              value={draft.kind}
              onChange={(value) => changeKind(value as AppKind)}
              options={[
                { value: 'service', label: '长期运行的服务' },
                { value: 'task', label: '一次性脚本任务' },
              ]}
            />
            <TextInput
              label="用途说明"
              name="description"
              className="col-span-full"
              value={draft.description}
              onChange={(value) => field('description', value)}
              maxLength={300}
              placeholder="这个应用用来做什么"
            />
            <TextInput
              label="准备命令（可选）"
              name="prepareCommand"
              className="col-span-full"
              multiline
              rows={2}
              value={draft.prepareCommand || ''}
              onChange={(value) => field('prepareCommand', value || null)}
              help="由云端守护进程先执行，成功后才启动应用。"
            />
            <TextInput
              label="云端启动命令"
              name="command"
              className="col-span-full"
              multiline
              rows={2}
              value={draft.command}
              onChange={(value) => field('command', value)}
              isRequired
              help="在云端 Linux 保持前台运行，不要加 & 或自行 nohup；后台管理由守护进程负责。"
            />
            <TextInput
              label="云端工作目录"
              name="directory"
              className="col-span-full"
              value={draft.workingDirectory}
              onChange={(value) => field('workingDirectory', value)}
              isRequired
            />
            {draft.kind === 'service' && (
              <>
                <PortInput
                  label="远端服务端口"
                  name="remotePort"
                  value={draft.remotePort}
                  onChange={(value) => field('remotePort', value)}
                />
                <PortInput
                  label="本机访问端口"
                  name="localPort"
                  value={draft.localPort}
                  onChange={(value) => field('localPort', value)}
                />
                <TextInput
                  label="健康检查路径"
                  name="healthPath"
                  className="col-span-full"
                  value={draft.healthPath || ''}
                  onChange={(value) => field('healthPath', value)}
                  isRequired
                  placeholder="/"
                />
              </>
            )}
            <TextInput
              label="需要保存的云端目录"
              name="saveDirectories"
              className="col-span-full"
              multiline
              value={directories}
              onChange={setDirectories}
              help="每行一个绝对路径。关机前需完成同步和目标端校验。"
            />
            <TextInput
              label="云盘保存目录"
              name="storageDirectory"
              className="col-span-full"
              value={draft.storageDirectory}
              onChange={(value) => field('storageDirectory', value)}
              isRequired
            />
          </div>
        </Modal.Body>
        <Modal.Footer>
          <Button variant="tertiary" onPress={close} isDisabled={saving}>
            取消
          </Button>
          <Button type="submit" variant="primary" isPending={saving}>
            {saving ? '保存中…' : '保存应用'}
          </Button>
        </Modal.Footer>
      </Form>
    </Dialog>
  );
}
