"""Explicit cloud QA: inspect idle state, verify a real external download and manifest execution."""
import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import urllib.error
import urllib.request

root = Path('/opt/cloud-deck')
request = json.load(sys.stdin)
manifest = request['manifest']
action = request['action']
if action == 'preflight':
    try:
        with urllib.request.urlopen('http://127.0.0.1:8188/queue', timeout=5) as response:
            queue = json.load(response)
        queue_state = {'running': len(queue['queue_running']), 'pending': len(queue['queue_pending'])}
    except urllib.error.URLError:
        queue_state = None
    state_path = root / 'state/applications.json'
    state = json.loads(state_path.read_text()) if state_path.exists() else {}
    print(json.dumps({'queue': queue_state, 'applications': state.get('applications', []),
                      'models': [{'target': resource['target'], 'present': Path(resource['target']).is_file()} for resource in manifest['resources']['files']]}))
elif action == 'download':
    specification = importlib.util.spec_from_file_location('lifecycle', root / 'cloud_lifecycle.py')
    lifecycle = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(lifecycle)
    # An isolated copy proves an actual transfer even when production models are cached.
    scratch = Path('/Volumes/CODEX_SCRATCH/tmp/cloud-deck-manifest')
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=scratch) as directory:
        resource = dict(min(manifest['resources']['files'], key=lambda item: item['size']))
        resource['target'] = str(Path(directory) / 'verified-model.pth')
        with contextlib.redirect_stdout(io.StringIO()):
            evidence = lifecycle.stage_resource(resource)
        evidence['sha256'] = hashlib.sha256(Path(resource['target']).read_bytes()).hexdigest()
        evidence['target'] = 'isolated-external-download-check'
        if evidence['source'] != 'download':
            raise RuntimeError('External source did not succeed; fallback was used')
        print(json.dumps(evidence))
elif action == 'evidence':
    state = json.loads((root / 'state/applications.json').read_text())
    recipe = next(recipe for recipe in state['recipes'] if recipe['id'] == 'comfyui')
    if recipe.get('manifest') != manifest:
        raise RuntimeError('Running application did not use the canonical manifest')
    for name, content in request['files'].items():
        if (root / 'state/packages/comfyui' / name).read_text() != content:
            raise RuntimeError('Deployed application script differs from the bundle')
    resources = json.loads((root / 'state/manifests/comfyui/resources-verified.json').read_text())
    print(json.dumps({'canonicalManifestApplied': True, 'applicationScriptsMatch': True, 'resources': resources,
                      'restoreRules': len(manifest['startup']['restore']), 'saveRules': len(manifest['shutdown']['save'])}))
else:
    raise ValueError('Unknown manifest check action')
