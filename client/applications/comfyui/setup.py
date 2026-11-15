#!/usr/bin/env python3
"""ComfyUI's own installer. Cloud Deck only runs this command and checks its exit code."""
import concurrent.futures
import hashlib
import importlib.metadata
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile

APPLICATION = Path('/opt/ComfyUI')
VIRTUAL_ENV = Path('/opt/comfy-venv')
MODELS = Path('/opt/comfy-models')
USER_DATA = Path('/opt/comfy-local')
BUILD = Path('/opt/comfy-build')


def run(arguments, directory=None, required=True):
    print('+ ' + ' '.join(map(str, arguments)), flush=True)
    return subprocess.run(list(map(str, arguments)), cwd=directory, check=required)


def install_git(target, repository, revision):
    if target.exists() and not (target / '.git').is_dir():
        raise RuntimeError(f'Existing directory is not a Git checkout: {target}')
    if not target.exists():
        run(['git', 'init', '-q', target])
        run(['git', '-C', target, 'remote', 'add', 'origin', repository])
    current = subprocess.run(['git', '-C', target, 'rev-parse', 'HEAD'], capture_output=True, text=True)
    if current.returncode == 0 and current.stdout.strip() == revision:
        return
    # Fetch the declared source directly; keep local edits if checkout cannot proceed.
    run(['git', '-C', target, 'fetch', '--depth', '1', repository, revision])
    run(['git', '-C', target, 'checkout', '-q', revision])


def install_archive(target, address):
    marker = target / '.cloud-deck-source'
    if marker.is_file() and marker.read_text() == address:
        return
    if target.exists():
        raise RuntimeError(f'Existing plugin directory has no matching source receipt: {target}')
    with tempfile.TemporaryDirectory(dir=BUILD) as temporary:
        directory = Path(temporary)
        archive = directory / 'plugin.archive'
        request = urllib.request.Request(address, headers={'User-Agent': 'Cloud-Deck-ComfyUI/1'})
        with urllib.request.urlopen(request, timeout=90) as response, archive.open('wb') as output:
            shutil.copyfileobj(response, output)
        extracted = directory / 'extracted'
        extracted.mkdir()
        if zipfile.is_zipfile(archive):
            with zipfile.ZipFile(archive) as package:
                for member in package.infolist():
                    relative = Path(member.filename)
                    if relative.is_absolute() or '..' in relative.parts or (member.external_attr >> 16) & 0o170000 == 0o120000:
                        raise RuntimeError('Unsafe archive entry')
                package.extractall(extracted)
        else:
            with tarfile.open(archive) as package:
                package.extractall(extracted, filter='data')
        entries = [entry for entry in extracted.iterdir() if entry.name != '__MACOSX']
        source = entries[0] if len(entries) == 1 and entries[0].is_dir() else extracted
        shutil.copytree(source, target)
        marker.write_text(address)


def link_data(source, target):
    """Adopt existing files without replacing different user content."""
    target.mkdir(parents=True, exist_ok=True)
    if source.is_symlink():
        if source.resolve() != target.resolve():
            raise RuntimeError(f'Unexpected data link: {source}')
        return
    if source.exists():
        for origin in source.rglob('*'):
            if origin.is_symlink():
                raise RuntimeError(f'Unexpected link in application data: {origin}')
            if origin.is_dir():
                continue
            destination = target / origin.relative_to(source)
            destination.parent.mkdir(parents=True, exist_ok=True)
            if destination.exists():
                if not destination.is_file() or digest(origin) != digest(destination):
                    raise RuntimeError(f'Existing data differs; both copies retained: {origin}')
            else:
                shutil.copy2(origin, destination)
        shutil.rmtree(source)
    source.symlink_to(target, target_is_directory=True)


def digest(filename):
    with filename.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').digest()


def dependency_line(value):
    value = value.split('#', 1)[0].strip()
    lowered = value.lower()
    if not value or value.startswith('-'):
        return None
    if lowered in ('torch', 'numpy', 'pillow', 'scipy') or lowered.startswith((
        'torch==', 'torch>=', 'torch<', 'torch ', 'torch\t', 'torchvision', 'torchaudio',
        'opencv-python', 'opencv-contrib-python', 'inference-cli', 'inference-gpu',
    )):
        return None
    return value


def install_plugins():
    nodes = APPLICATION / 'custom_nodes'
    nodes.mkdir(exist_ok=True)
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
        jobs = []
        jobs.append(executor.submit(install_git, nodes / 'ComfyLiterals', 'https://github.com/M1kep/ComfyLiterals', 'bdddb08ca82d90d75d97b1d437a652e0284a32ac'))
        jobs.append(executor.submit(install_git, nodes / 'ComfyUI-0246.disabled', 'https://github.com/Trung0246/ComfyUI-0246', 'd09cb3fe8266e4731316955a99c28dd721ee26cd'))
        jobs.append(executor.submit(install_archive, nodes / 'ComfyUI-Crystools', 'https://cdn.comfy.org/crystian/ComfyUI-Crystools/1.27.4/node.zip'))
        jobs.append(executor.submit(install_git, nodes / 'ComfyUI-Manager', 'https://github.com/Comfy-Org/ComfyUI-Manager', '00a5115267378c75f3eb52dc21a07099722a9922'))
        jobs.append(executor.submit(install_git, nodes / 'ComfyUI-PainterI2V', 'https://github.com/princepainter/ComfyUI-PainterI2V', 'c46e480c35d9a54e70139e354b1fc2975d0b6805'))
        jobs.append(executor.submit(install_git, nodes / 'ComfyUI-QwenVL', 'https://github.com/1038lab/ComfyUI-QwenVL', 'c522c43b15618a4d5c92b2500105ee2a65527f95'))
        jobs.append(executor.submit(install_git, nodes / 'ComfyUI-WanMoeKSampler', 'https://github.com/stduhpf/ComfyUI-WanMoeKSampler', 'ee8fa8ffb356dc5d9f456a0cdb211e60e543cb59'))
        jobs.append(executor.submit(install_archive, nodes / 'ComfyUI-WanVideoWrapper', 'https://cdn.comfy.org/kijai/ComfyUI-WanVideoWrapper/1.3.7/node.zip'))
        jobs.append(executor.submit(install_git, nodes / 'ComfyUI-llama-cpp_vlm', 'https://github.com/lihaoyun6/ComfyUI-llama-cpp_vlm', '52faab1d1c5eb1bb64fd65c09a73da6ed65caec5'))
        jobs.append(executor.submit(install_git, nodes / 'ComfyUI_Comfyroll_CustomNodes', 'https://github.com/Suzie1/ComfyUI_Comfyroll_CustomNodes', 'd78b780ae43fcf8c6b7c6505e6ffb4584281ceca'))
        jobs.append(executor.submit(install_git, nodes / 'Comfyui-Memory_Cleanup', 'https://github.com/LAOGOU-666/Comfyui-Memory_Cleanup', '58de13a6090e04408e343501ff8902c034d9f518'))
        jobs.append(executor.submit(install_archive, nodes / 'cg-use-everywhere', 'https://cdn.comfy.org/chrisgoringe/cg-use-everywhere/7.3.0/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'comfy-mtb', 'https://cdn.comfy.org/mel/comfy-mtb/0.5.4/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui-animatediff-evolved', 'https://cdn.comfy.org/kosinkadink/comfyui-animatediff-evolved/1.5.6/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui-cogvideoxwrapper', 'https://cdn.comfy.org/kijai/comfyui-cogvideoxwrapper/1.5.1/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui-custom-scripts', 'https://cdn.comfy.org/pythongosssss/comfyui-custom-scripts/1.2.5/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui-easy-use', 'https://cdn.comfy.org/yolain/comfyui-easy-use/1.3.4/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui-florence2', 'https://cdn.comfy.org/kijai/comfyui-florence2/1.0.6/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui-frame-interpolation', 'https://cdn.comfy.org/fannovel16/comfyui-frame-interpolation/1.0.7/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui-impact-pack', 'https://cdn.comfy.org/drltdata/comfyui-impact-pack/8.25.1/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui-inpaint-cropandstitch', 'https://cdn.comfy.org/lquesada/comfyui-inpaint-cropandstitch/2.1.7/node.zip'))
        jobs.append(executor.submit(install_git, nodes / 'comfyui-kjnodes', 'https://github.com/kijai/ComfyUI-KJNodes', '3f20054214fec9f9234fd3841ae6f1e4287948f6'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui-mixlab-nodes', 'https://cdn.comfy.org/shadow/comfyui-mixlab-nodes/0.46.0/node.tar.gz'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui-mxtoolkit', 'https://cdn.comfy.org/smirnov/comfyui-mxtoolkit/0.9.92/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui-videohelpersuite', 'https://cdn.comfy.org/kosinkadink/comfyui-videohelpersuite/1.7.7/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui-wd14-tagger', 'https://cdn.comfy.org/pythongosssss/comfyui-wd14-tagger/1.0.1/node.zip'))
        jobs.append(executor.submit(install_git, nodes / 'comfyui_controlnet_aux', 'https://github.com/Fannovel16/comfyui_controlnet_aux', 'cc6b232f4a47f0cdf70f4e1bfa24b74bd0d75bf1'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui_custom_nodes_alekpet', 'https://cdn.comfy.org/alekpet/comfyui_custom_nodes_alekpet/1.0.88/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui_essentials', 'https://cdn.comfy.org/matteo/comfyui_essentials/1.1.0/node.tar.gz'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui_layerstyle', 'https://cdn.comfy.org/chflame163/comfyui_layerstyle/1.0.90/node.tar.gz'))
        jobs.append(executor.submit(install_archive, nodes / 'comfyui_ultimatesdupscale', 'https://cdn.comfy.org/ssit/comfyui_ultimatesdupscale/1.5.1/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'derfuu_comfyui_moddednodes', 'https://cdn.comfy.org/derfuu/derfuu_comfyui_moddednodes/1.0.1/node.tar.gz'))
        jobs.append(executor.submit(install_archive, nodes / 'rgthree-comfy', 'https://cdn.comfy.org/rgthree/rgthree-comfy/1.0.2510052058/node.zip'))
        jobs.append(executor.submit(install_archive, nodes / 'was-ns', 'https://cdn.comfy.org/drltdata/was-ns/3.0.1/node.zip'))
        for future in concurrent.futures.as_completed(jobs):
            future.result()


def install_dependencies():
    if not (VIRTUAL_ENV / 'bin/python').exists():
        if shutil.which('uv'):
            run(['uv', 'venv', '--system-site-packages', '--seed', VIRTUAL_ENV])
        else:
            run([sys.executable, '-m', 'venv', '--system-site-packages', VIRTUAL_ENV])
    executable = VIRTUAL_ENV / 'bin/python'
    constraints = ['aiohttp>=3.11.8']
    for package in ('torch', 'torchvision', 'torchaudio'):
        try:
            constraints.append(f'{package}=={importlib.metadata.version(package)}')
        except importlib.metadata.PackageNotFoundError:
            pass
    constraint_file = BUILD / 'constraints.txt'
    constraint_file.write_text('\n'.join(constraints) + '\n')
    pip_command = [executable, '-m', 'pip', 'install', '--cache-dir', BUILD / 'pip', '-c', constraint_file]
    run([*pip_command, '-r', APPLICATION / 'requirements.txt'])
    declared = [
        'deepdiff',
        'torch',
        'numpy',
        'Pillow',
        'pynvml',
        'py-cpuinfo',
        'accelerate >= 1.2.1',
        'diffusers >= 0.33.0',
        'peft >= 0.17.0',
        'ftfy',
        'gguf >= 0.17.1',
        'pyloudnorm',
        'qrcode',
        'cachetools',
        'onnxruntime-gpu',
        'requirements-parserx',
        'rembg',
        'imageio_ffmpeg',
        'rich',
        'rich_argparse',
        'matplotlib',
        'pillow',
        'huggingface_hub',
        'diffusers>=0.31.0',
        'accelerate>=0.33.0',
        'diffusers',
        'accelerate',
        'clip_interrogator>=0.6.0',
        'sentencepiece',
        'lark',
        'onnxruntime',
        'spandrel',
        'opencv-python-headless',
        'peft',
        'transformers>=4.39.0,!=4.50.*',
        'segment-anything',
        'scikit-image',
        'piexif',
        'transformers',
        'GitPython',
        'scipy>=1.11.4',
        'pyOpenSSL',
        'watchdog',
        'openai',
        'simple-lama-inpainting',
        'clip-interrogator==0.6.0',
        'transformers>=4.36.0',
        'lark-parser',
        'imageio-ffmpeg',
        'rembg[gpu]',
        'omegaconf==2.3.0',
        'Pillow>=9.5.0',
        'einops==0.7.0',
        'trimesh>=4.0.5',
        'huggingface-hub',
        'opencv-python',
        'numba',
        'colour-science',
        'pixeloe',
        'Scipy',
        'scikit_image',
        'scikit_learn',
        'opencv-contrib-python',
        'pymatting',
        'segment_anything',
        'timm',
        'addict',
        'yapf',
        'wget',
        'mediapipe',
        'loguru',
        'typer_config',
        'fastapi',
        'google-generativeai',
        'omegaconf',
        'tqdm',
        'kornia',
        'image-reward',
        'ultralytics',
        'blend_modes',
        'blind-watermark',
        'pyzbar',
        'transparent-background',
        'bitsandbytes',
        'torchscale',
        'wandb',
        'hydra-core',
        'psd-tools',
        'inference-cli[yolo-world]',
        'inference-gpu[yolo-world]',
        'iopath',
        'cmake',
        'fairscale>=0.4.4',
        'git+https://github.com/ltdrdata/img2texture.git',
        'git+https://github.com/ltdrdata/cstr',
        'gitpython',
        'imageio',
        'joblib',
        'numba>=0.62',
        'opencv-python-headless[ffmpeg]<=4.7.0.72',
        'pilgram',
        'git+https://github.com/ltdrdata/ffmpy.git',
        'scikit-image>=0.20.0',
        'scikit-learn',
        'scipy',
        'timm>=0.4.12',
    ]
    for requirement in sorted((APPLICATION / 'custom_nodes').glob('*/requirements.txt')):
        if requirement.parent.name.endswith('.disabled'):
            continue
        declared.extend(requirement.read_text(errors='replace').splitlines())
    dependencies = sorted({cleaned for value in declared if (cleaned := dependency_line(value))})
    requirements = BUILD / 'plugin-requirements.txt'
    requirements.write_text('\n'.join(dependencies) + '\n')
    if dependencies and run([*pip_command, '-r', requirements], required=False).returncode:
        for dependency in dependencies:
            if run([*pip_command, dependency], required=False).returncode:
                print(f'Plugin dependency unavailable: {dependency}', flush=True)
    # Core requirements must still resolve after optional plugin dependencies.
    run([*pip_command, '-r', APPLICATION / 'requirements.txt'])
    for package in ('bitsandbytes', 'sageattention'):
        run([*pip_command, package], required=False)


def main():
    BUILD.mkdir(parents=True, exist_ok=True)
    os.environ['TMPDIR'] = str(BUILD)
    os.environ['XDG_CACHE_HOME'] = str(BUILD / 'cache')
    os.environ['UV_CACHE_DIR'] = str(BUILD / 'uv')
    tempfile.tempdir = str(BUILD)
    if not shutil.which('ffmpeg'):
        run(['apt-get', 'update', '-qq'])
        run(['apt-get', 'install', '-y', '-qq', 'ffmpeg'], required=False)
    install_git(APPLICATION, 'https://github.com/Comfy-Org/ComfyUI', 'b1693ecba9f5b65f8c80ab36b195ab963ec92413')
    signature = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    receipt = BUILD / 'setup-completed'
    if not receipt.is_file() or receipt.read_text() != signature or not (VIRTUAL_ENV / 'bin/python').is_file():
        install_plugins()
        install_dependencies()
        receipt.write_text(signature)
    link_data(APPLICATION / 'models', MODELS)
    for name in ('user', 'input', 'output'):
        link_data(APPLICATION / name, USER_DATA / name)
    print('ComfyUI environment ready', flush=True)


if __name__ == '__main__':
    main()
