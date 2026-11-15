import { useState } from 'react';
import type { FormEvent } from 'react';
import { Form, Modal } from '@heroui/react';
import type { ApplicationManifest } from '../manifest.ts';
import { parseApplicationManifest } from '../manifest.ts';
import { Button, Dialog, Notice, TextInput } from './ui.tsx';

export function ManifestEditor({
  initial,
  readOnly = false,
  save,
  close,
}: {
  initial: ApplicationManifest;
  readOnly?: boolean;
  save: (manifest: ApplicationManifest) => Promise<void>;
  close: () => void;
}) {
  const [text, setText] = useState(JSON.stringify(initial, null, 2));
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  function format() {
    try {
      setText(JSON.stringify(parseApplicationManifest(text), null, 2));
      setError('');
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (readOnly || saving) return;
    setSaving(true);
    setError('');
    try {
      await save(parseApplicationManifest(text));
      close();
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  }
  return (
    <Dialog
      title={readOnly ? '应用清单 · manifest.json' : '编辑应用清单'}
      close={close}
      locked={saving}
    >
      <Form onSubmit={submit} className="cd-modal-form">
        <Modal.Body>
          <p className="mb-4 text-sm text-muted">
            清单决定运行方式、启动恢复、准备命令和关机保存。资源按 sources 顺序尝试，大小和 SHA-256
            通过后才使用。 details 可填写只读 Markdown 说明，支持标题、列表和表格。
          </p>
          {error && (
            <div role="alert">
              <Notice tone="bad">{error}</Notice>
            </div>
          )}
          <TextInput
            label="manifest.json"
            name="manifest"
            multiline
            rows={20}
            className="font-mono text-xs"
            value={text}
            onChange={setText}
            isReadOnly={readOnly}
            help={
              readOnly
                ? '内置清单随客户端提供，可选择并复制内容。'
                : '保存清单后，在应用页面部署；保存本身不会执行命令。'
            }
          />
        </Modal.Body>
        <Modal.Footer>
          {!readOnly && (
            <Button variant="outline" onPress={format} isDisabled={saving}>
              校验并格式化
            </Button>
          )}
          <Button variant="tertiary" onPress={close} isDisabled={saving}>
            {readOnly ? '关闭' : '取消'}
          </Button>
          {!readOnly && (
            <Button type="submit" isPending={saving}>
              保存清单
            </Button>
          )}
        </Modal.Footer>
      </Form>
    </Dialog>
  );
}
