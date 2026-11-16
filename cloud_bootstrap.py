#!/usr/bin/env python3
"""Runs on the remote Colab Ubuntu Linux VM (as root) to restore or build ComfyUI.

ponytail: venv --system-site-packages + tar.zst cache on Google Drive;
1st boot builds from upstream GitHub/Comfy CDN, 2nd+ boot extracts in seconds to local NVMe.
"""
import concurrent.futures
import hashlib
import importlib.metadata as meta
import json
import os
import pathlib
import re
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import urllib.request
import zipfile

DRIVE_ROOT = pathlib.Path("/content/drive/MyDrive/ComfyUI_Cloud")
APP_DIR = pathlib.Path("/opt/ComfyUI")
VENV_DIR = pathlib.Path("/opt/comfy-venv")
LOCAL_MODELS = pathlib.Path("/opt/comfy-models")
LOCAL_DATA = pathlib.Path("/opt/comfy-local")
SYNC_BIN = pathlib.Path("/usr/local/bin/comfy-sync-drive")
LOG_FILE = pathlib.Path("/var/log/comfyui.log")
PID_FILE = pathlib.Path("/var/run/comfyui.pid")

MODEL_SUBDIRS = [
    "checkpoints", "loras", "vae", "clip", "text_encoders", "diffusion_models",
    "controlnet", "upscale_models", "clip_vision", "embeddings", "style_models",
    "sams", "ultralytics", "LLM", "inpaint", "lama", "rembg", "vae_approx",
]


def _run(cmd: list[str], cwd: pathlib.Path | None = None, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, cwd=str(cwd) if cwd else None, check=check, text=True, capture_output=True)


def _ensure_drive_dirs(require_mount: bool = False) -> None:
    if require_mount and not os.path.ismount("/content/drive"):
        raise RuntimeError("Google Drive is not mounted; managed application startup is blocked.")
    if not pathlib.Path("/content/drive/MyDrive").is_dir():
        raise RuntimeError("/content/drive/MyDrive is not mounted yet.")
    for sub in ("input", "output", "user", "cache", "cache/pip"):
        (DRIVE_ROOT / sub).mkdir(parents=True, exist_ok=True)
    for msub in MODEL_SUBDIRS:
        (DRIVE_ROOT / "models" / msub).mkdir(parents=True, exist_ok=True)


def _extract_archive_into(archive_path: pathlib.Path, dest_dir: pathlib.Path) -> None:
    with tempfile.TemporaryDirectory() as tmpd:
        tmp_root = pathlib.Path(tmpd)
        if zipfile.is_zipfile(archive_path):
            with zipfile.ZipFile(archive_path) as zf:
                zf.extractall(tmp_root)
        else:
            with tarfile.open(archive_path) as tf:
                tf.extractall(tmp_root)
        entries = [p for p in tmp_root.iterdir() if p.name != "__MACOSX"]
        src = entries[0] if len(entries) == 1 and entries[0].is_dir() else tmp_root
        if dest_dir.exists():
            shutil.rmtree(dest_dir)
        shutil.copytree(src, dest_dir)


def _install_one_plugin(plugin: dict, custom_nodes_dir: pathlib.Path) -> str:
    target = custom_nodes_dir / plugin["directory"]
    if target.exists():
        shutil.rmtree(target)
    if plugin["source"] == "git":
        _run(["git", "init", "-q", str(target)])
        _run(["git", "-C", str(target), "remote", "add", "origin", plugin["repository"]])
        _run(["git", "-C", str(target), "fetch", "--depth", "1", "origin", plugin["commit"]])
        _run(["git", "-C", str(target), "checkout", "-q", "FETCH_HEAD"])
    else:
        url = plugin["download_url"]
        suffix = ".tar.gz" if url.endswith(".tar.gz") else ".zip"
        with tempfile.NamedTemporaryFile(suffix=suffix, delete=False) as tf:
            tmp_file = pathlib.Path(tf.name)
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "ComfyUI-Colab-Bootstrap/1.0"})
            with urllib.request.urlopen(req, timeout=60) as r, open(tmp_file, "wb") as out:
                shutil.copyfileobj(r, out)
            _extract_archive_into(tmp_file, target)
        finally:
            tmp_file.unlink(missing_ok=True)
    return plugin["directory"]


def _filter_plugin_req_line(raw: str) -> str | None:
    line = raw.split("#", 1)[0].strip()
    if not line or line.startswith(("-", "--")):
        return None
    low = line.lower()
    # Protect Colab's preinstalled CUDA PyTorch & modern OpenCV/aiohttp
    skip_prefixes = (
        "torch==", "torch>=", "torch<", "torch ", "torch\t",
        "torchvision", "torchaudio",
        "opencv-python", "opencv-contrib-python",
        "inference-cli", "inference-gpu",
    )
    if low in ("torch", "numpy", "pillow", "scipy") or low.startswith(skip_prefixes):
        return None
    return line


def _build_fresh(manifest: dict, cache_tar: pathlib.Path) -> None:
    apt_env = {**os.environ, "DEBIAN_FRONTEND": "noninteractive"}
    if shutil.which("zstd") is None or shutil.which("aria2c") is None:
        subprocess.run(["apt-get", "update", "-qq"], env=apt_env, check=False)
        subprocess.run(["apt-get", "install", "-y", "-qq", "zstd", "aria2", "ffmpeg"], env=apt_env, check=False)

    if APP_DIR.exists():
        shutil.rmtree(APP_DIR)
    if VENV_DIR.exists():
        shutil.rmtree(VENV_DIR)

    comfy = manifest["comfyui"]
    _run(["git", "init", "-q", str(APP_DIR)])
    _run(["git", "-C", str(APP_DIR), "remote", "add", "origin", comfy["repository"]])
    _run(["git", "-C", str(APP_DIR), "fetch", "--depth", "1", "origin", comfy["commit"]])
    _run(["git", "-C", str(APP_DIR), "checkout", "-q", "FETCH_HEAD"])

    custom_nodes_dir = APP_DIR / "custom_nodes"
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        futs = [pool.submit(_install_one_plugin, p, custom_nodes_dir) for p in manifest["plugins"]]
        for f in concurrent.futures.as_completed(futs):
            f.result()

    uv_bin = shutil.which("uv")
    if uv_bin:
        _run([uv_bin, "venv", "--system-site-packages", "--seed", str(VENV_DIR)])
    else:
        venv_rc = subprocess.run([sys.executable, "-m", "venv", "--system-site-packages", str(VENV_DIR)]).returncode
        if venv_rc != 0:
            subprocess.run(["apt-get", "install", "-y", "-qq", "python3-venv"], env=apt_env, check=False)
            _run([sys.executable, "-m", "venv", "--system-site-packages", str(VENV_DIR)])

    vpy = str(VENV_DIR / "bin" / "python")
    pip_cache = str(DRIVE_ROOT / "cache" / "pip")

    # Pin preinstalled CUDA torch stack so plugin deps never overwrite it
    constraints = ["aiohttp>=3.11.8"]
    for pkg in ("torch", "torchvision", "torchaudio"):
        try:
            constraints.append(f"{pkg}=={meta.version(pkg)}")
        except meta.PackageNotFoundError:
            pass
    cfile = pathlib.Path("/tmp/comfy-constraints.txt")
    cfile.write_text("\n".join(constraints) + "\n")

    plugin_reqs: list[str] = []
    seen: set[str] = set()
    for p in manifest["plugins"]:
        if p.get("disabled"):
            continue
        for dep in p.get("dependencies", []):
            cleaned = _filter_plugin_req_line(dep)
            if cleaned and cleaned not in seen:
                seen.add(cleaned)
                plugin_reqs.append(cleaned)
        req_txt = custom_nodes_dir / p["directory"] / "requirements.txt"
        if req_txt.is_file():
            for line in req_txt.read_text(errors="replace").splitlines():
                cleaned = _filter_plugin_req_line(line)
                if cleaned and cleaned not in seen:
                    seen.add(cleaned)
                    plugin_reqs.append(cleaned)

    def pip_cmd(extra_args: list[str]) -> list[str]:
        # Standard pip respects installed local +cu130 CUDA torch versions in --system-site-packages
        return [vpy, "-m", "pip", "install", "-q", "--cache-dir", pip_cache, "-c", str(cfile)] + extra_args

    if plugin_reqs:
        pfile = pathlib.Path("/tmp/comfy-plugins-reqs.txt")
        pfile.write_text("\n".join(plugin_reqs) + "\n")
        batch = subprocess.run(pip_cmd(["-r", str(pfile)]), text=True, capture_output=True)
        if batch.returncode != 0:
            for req_item in plugin_reqs:
                subprocess.run(pip_cmd([req_item]), check=False, capture_output=True)

    # Core ComfyUI v0.45.4 requirements + CUDA 13 compatible bitsandbytes/sageattention
    _run(pip_cmd(["-r", str(APP_DIR / "requirements.txt")]))
    subprocess.run(pip_cmd(["-U", "bitsandbytes"]), check=False, capture_output=True)
    subprocess.run(pip_cmd(["sageattention"]), check=False, capture_output=True)

    # Remove placeholder dirs before archiving so symlinks attach cleanly on restore
    for d in ("models", "input", "output", "user"):
        p = APP_DIR / d
        if p.is_symlink():
            p.unlink()
        elif p.exists():
            shutil.rmtree(p)

    tmp_tar = cache_tar.with_suffix(".tar.zst.tmp")
    tmp_tar.unlink(missing_ok=True)
    _run(["tar", "--zstd", "-cf", str(tmp_tar), "/opt/ComfyUI", "/opt/comfy-venv"])
    os.replace(tmp_tar, cache_tar)


_SYNC_DRIVE_SCRIPT = """#!/usr/bin/env python3
import json, os, pathlib, shutil, sys, time

DRIVE_ROOT = pathlib.Path("/content/drive/MyDrive/ComfyUI_Cloud")
LOCAL_DATA = pathlib.Path("/opt/comfy-local")
BASELINE_FILE = LOCAL_DATA / ".user_workflows_baseline.json"


def _net_tx_bytes() -> int:
    total = 0
    for line in pathlib.Path("/proc/net/dev").read_text().splitlines()[2:]:
        if ":" not in line:
            continue
        iface, rest = line.split(":", 1)
        if iface.strip() == "lo":
            continue
        cols = rest.split()
        if len(cols) >= 9:
            total += int(cols[8])
    return total


def sync_once(flush: bool = False) -> dict:
    if not (DRIVE_ROOT.parent.is_dir() and LOCAL_DATA.is_dir()):
        return {"synced": 0, "removed": 0, "flushed": False, "skipped": True}
    synced = 0
    removed = 0
    for sub in ("user", "output", "input"):
        src_dir = LOCAL_DATA / sub
        dst_dir = DRIVE_ROOT / sub
        if not src_dir.is_dir():
            continue
        dst_dir.mkdir(parents=True, exist_ok=True)
        for src in src_dir.rglob("*"):
            if not src.is_file() or src.name.endswith(".sync.part"):
                continue
            rel = src.relative_to(src_dir)
            dst = dst_dir / rel
            s_st = src.stat()
            if dst.is_file():
                d_st = dst.stat()
                if d_st.st_size == s_st.st_size and s_st.st_mtime <= d_st.st_mtime + 0.5:
                    continue
            dst.parent.mkdir(parents=True, exist_ok=True)
            tmp = dst.with_name(dst.name + ".sync.part")
            with open(src, "rb") as fsrc, open(tmp, "wb") as fdst:
                shutil.copyfileobj(fsrc, fdst, length=8 * 1024 * 1024)
                fdst.flush()
                os.fsync(fdst.fileno())
            os.replace(tmp, dst)
            try:
                os.utime(dst, (s_st.st_atime, s_st.st_mtime))
            except OSError:
                pass
            synced += 1

    # Propagate intentional workflow JSON deletions made during this session (never touch output)
    if BASELINE_FILE.is_file():
        try:
            baseline = set(json.loads(BASELINE_FILE.read_text()))
        except Exception:
            baseline = set()
        local_wf = LOCAL_DATA / "user" / "default" / "workflows"
        drive_wf = DRIVE_ROOT / "user" / "default" / "workflows"
        for rel_str in baseline:
            if not (local_wf / rel_str).exists() and (drive_wf / rel_str).is_file():
                (drive_wf / rel_str).unlink(missing_ok=True)
                removed += 1

    flushed = False
    if flush:
        os.sync()
        quiet_ticks = 0
        prev_tx = _net_tx_bytes()
        for _ in range(60):
            time.sleep(1.0)
            cur_tx = _net_tx_bytes()
            delta = max(0, cur_tx - prev_tx)
            prev_tx = cur_tx
            if delta <= 32768:
                quiet_ticks += 1
                if quiet_ticks >= 2:
                    flushed = True
                    break
            else:
                quiet_ticks = 0
    return {"synced": synced, "removed": removed, "flushed": flushed}


def main() -> None:
    if "--daemon" in sys.argv:
        while True:
            time.sleep(20)
            try:
                sync_once(flush=False)
            except Exception:
                pass
    else:
        res = sync_once(flush="--flush" in sys.argv)
        print(json.dumps(res, ensure_ascii=False))


if __name__ == "__main__":
    main()
"""


def _install_and_start_sync_daemon() -> None:
    SYNC_BIN.write_text(_SYNC_DRIVE_SCRIPT)
    os.chmod(SYNC_BIN, 0o755)
    if subprocess.run(["pgrep", "-f", "/usr/local/bin/comfy-sync-drive --daemon"], capture_output=True).returncode != 0:
        subprocess.Popen(
            [sys.executable, str(SYNC_BIN), "--daemon"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )


def _stage_local_userdata(preserve_local: bool = False) -> None:
    """Restore user/input files; managed restarts never overwrite instance-local changes."""
    for sub in ("user", "input", "output"):
        (LOCAL_DATA / sub).mkdir(parents=True, exist_ok=True)
    restore_directories = () if os.environ.get("CLOUD_DECK_LIFECYCLE_MANAGED") == "1" else ("user", "input")
    for sub in restore_directories:
        src_dir = DRIVE_ROOT / sub
        dst_dir = LOCAL_DATA / sub
        if src_dir.is_dir():
            for src in src_dir.rglob("*"):
                if not src.is_file() or src.name.endswith(".sync.part") or '.cloud-deck-backups' in src.relative_to(src_dir).parts:
                    continue
                rel = src.relative_to(src_dir)
                dst = dst_dir / rel
                s_st = src.stat()
                if dst.is_file():
                    if preserve_local:
                        continue
                    d_st = dst.stat()
                    if d_st.st_size == s_st.st_size and abs(d_st.st_mtime - s_st.st_mtime) <= 1.0:
                        continue
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(src, dst)

    wf_dir = LOCAL_DATA / "user" / "default" / "workflows"
    baseline = [str(p.relative_to(wf_dir)) for p in wf_dir.rglob("*.json") if p.is_file()] if wf_dir.is_dir() else []
    (LOCAL_DATA / ".user_workflows_baseline.json").write_text(json.dumps(sorted(baseline), ensure_ascii=False))

    for d in ("user", "input", "output"):
        p = APP_DIR / d
        if preserve_local and p.exists() and not p.is_symlink() and (not p.is_dir() or any(p.iterdir())):
            raise RuntimeError(f"Refusing to replace existing application data: {p}")
        if p.is_symlink() or p.is_file():
            p.unlink()
        elif p.exists():
            shutil.rmtree(p)
        p.symlink_to(LOCAL_DATA / d)


def _all_dirs_are_local() -> bool:
    try:
        if not (APP_DIR / "models").is_symlink() or not (APP_DIR / "models").resolve().is_relative_to(LOCAL_MODELS):
            return False
        for d in ("user", "input", "output"):
            link = APP_DIR / d
            if not link.is_symlink() or not link.resolve().is_relative_to(LOCAL_DATA):
                return False
        return True
    except OSError:
        return False


def _build_inventory_snapshot(manifest_models: list[dict], extra_drive_files: dict[str, int], progress_override: dict[str, int] | None = None) -> dict:
    merged: dict[str, dict] = {}
    for item in manifest_models:
        rel = item["path"].lstrip("/")
        merged[rel] = {"size": int(item.get("size") or 0), "source": "hf"}
    for rel, sz in extra_drive_files.items():
        if rel not in merged:
            merged[rel] = {"size": sz, "source": "drive"}

    files = []
    for rel in sorted(merged):
        info = merged[rel]
        sz = info["size"]
        dest = LOCAL_MODELS / rel
        part = dest.with_name(dest.name + ".part")
        if dest.is_file() and (sz == 0 or dest.stat().st_size == sz) and dest.stat().st_size > 0:
            actual = dest.stat().st_size
            st, cb = "ready", actual
            sz = actual
        elif progress_override and rel in progress_override:
            st, cb = "copying", min(sz, progress_override[rel]) if sz > 0 else progress_override[rel]
        elif part.is_file():
            st, cb = "copying", min(sz, part.stat().st_size) if sz > 0 else part.stat().st_size
        else:
            st, cb = "pending", 0
        files.append({"path": rel, "size": sz, "status": st, "copiedBytes": cb, "source": info["source"]})

    wf_local = LOCAL_DATA / "user" / "default" / "workflows"
    wf_drive = DRIVE_ROOT / "user" / "default" / "workflows"
    wf_dir = wf_local if wf_local.is_dir() else wf_drive
    wf_count = sum(1 for p in wf_dir.glob("*.json") if not p.name.startswith(".")) if wf_dir.is_dir() else None
    return {
        "root": "Hugging Face 直拉本地盘 + Google Drive 自定义模型",
        "observedAt": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "workflowCount": wf_count,
        "files": files,
    }


def _scan_drive_extra_models(manifest_paths: set[str]) -> dict[str, int]:
    src_root = DRIVE_ROOT / "models"
    extras: dict[str, int] = {}
    if src_root.is_dir():
        for src in sorted(src_root.rglob("*")):
            if not src.is_file():
                continue
            rel = str(src.relative_to(src_root))
            if rel not in manifest_paths:
                extras[rel] = src.stat().st_size
    return extras


def _write_status(payload: dict) -> None:
    tmp = STATUS_FILE.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(payload, ensure_ascii=False))
    os.replace(tmp, STATUS_FILE)


def _download_hf_parallel(url: str, headers: dict, dest_part: pathlib.Path, exp_size: int | None, on_progress) -> None:
    req_headers = {"User-Agent": "ComfyUI-Colab-Bootstrap/2.0", **(headers or {})}
    probe_req = urllib.request.Request(url, headers={**req_headers, "Range": "bytes=0-0"})
    with urllib.request.urlopen(probe_req, timeout=45) as resp:
        final_url = resp.geturl()
        code = resp.status
        cr = resp.headers.get("Content-Range", "")
        cl = resp.headers.get("Content-Length")
        if code == 206 and "/" in cr:
            total = int(cr.rsplit("/", 1)[1])
        elif exp_size:
            total = exp_size
        elif cl:
            total = int(cl)
        else:
            total = 0

    if code != 206 or total < 32 * 1024 * 1024:
        req = urllib.request.Request(final_url, headers=req_headers)
        done = 0
        with urllib.request.urlopen(req, timeout=120) as r, open(dest_part, "wb") as out:
            while buf := r.read(4 * 1024 * 1024):
                out.write(buf)
                done += len(buf)
                on_progress(done)
            out.flush()
            os.fsync(out.fileno())
        return

    fd = os.open(dest_part, os.O_CREAT | os.O_TRUNC | os.O_RDWR, 0o644)
    try:
        os.ftruncate(fd, total)
        chunk_size = 64 * 1024 * 1024
        ranges = [(s, min(s + chunk_size - 1, total - 1)) for s in range(0, total, chunk_size)]
        lock = threading.Lock()
        done_bytes = 0
        last_emit = 0.0

        def fetch_range(bounds: tuple[int, int]) -> None:
            nonlocal done_bytes, last_emit
            start, end = bounds
            for attempt in range(4):
                pos = start
                try:
                    rreq = urllib.request.Request(final_url, headers={**req_headers, "Range": f"bytes={start}-{end}"})
                    with urllib.request.urlopen(rreq, timeout=90) as resp:
                        if resp.status != 206:
                            raise RuntimeError(f"Expected HTTP 206, got {resp.status}")
                        while buf := resp.read(4 * 1024 * 1024):
                            os.pwrite(fd, buf, pos)
                            pos += len(buf)
                            with lock:
                                done_bytes += len(buf)
                                now = time.monotonic()
                                if now - last_emit >= 1.2:
                                    last_emit = now
                                    on_progress(done_bytes)
                    if pos != end + 1:
                        raise RuntimeError(f"Incomplete range {start}-{end}: got {pos - start}")
                    return
                except Exception:
                    if attempt == 3:
                        raise
                    with lock:
                        done_bytes = max(0, done_bytes - (pos - start))
                    time.sleep(1.0 * (attempt + 1))

        with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
            futs = [pool.submit(fetch_range, b) for b in ranges]
            for f in concurrent.futures.as_completed(futs):
                f.result()
        os.fsync(fd)
        on_progress(total)
    finally:
        os.close(fd)


def _sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while block := stream.read(8 * 1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def _stage_all_models_local(manifest_models: list[dict], cache_tar: pathlib.Path) -> tuple[list[str], list[str], dict]:
    """Download manifest models straight from HF (16x Range) to local NVMe, plus any extra custom models on Drive."""
    LOCAL_MODELS.mkdir(parents=True, exist_ok=True)
    for msub in MODEL_SUBDIRS:
        (LOCAL_MODELS / msub).mkdir(parents=True, exist_ok=True)

    if os.environ.get("CLOUD_DECK_LIFECYCLE_MANAGED") == "1":
        # The generic manifest runner already verified and staged these resources.
        for resource in manifest_models:
            target = LOCAL_MODELS / resource["path"]
            if not target.is_file() or target.stat().st_size != resource["size"]:
                raise RuntimeError("Manifest resource is not ready: " + str(target))
        model_link = APP_DIR / "models"
        if model_link.is_symlink() or model_link.is_file():
            model_link.unlink()
        elif model_link.exists():
            # Keep bundled placeholder files and any user models when adopting the local model root.
            for existing in sorted(model_link.rglob("*")):
                if existing.is_symlink():
                    raise RuntimeError("Existing model tree contains a symbolic link: " + str(existing))
                destination = LOCAL_MODELS / existing.relative_to(model_link)
                if existing.is_dir():
                    destination.mkdir(parents=True, exist_ok=True)
                elif existing.is_file():
                    if destination.exists():
                        if not destination.is_file() or existing.stat().st_size != destination.stat().st_size or _sha256(existing) != _sha256(destination):
                            raise RuntimeError("Conflicting model file was preserved: " + str(existing))
                    else:
                        destination.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copy2(existing, destination)
                else:
                    raise RuntimeError("Existing model tree contains a non-regular file")
            shutil.rmtree(model_link)
        model_link.symlink_to(LOCAL_MODELS)
        return [], [], _build_inventory_snapshot(manifest_models, _scan_drive_extra_models({resource["path"] for resource in manifest_models}))

    manifest_paths = {m["path"].lstrip("/") for m in manifest_models}
    extra_drive = _scan_drive_extra_models(manifest_paths)
    progress_map: dict[str, int] = {}

    def emit(current_rel: str | None = None) -> dict:
        inv = _build_inventory_snapshot(manifest_models, extra_drive, progress_map)
        payload = {"status": "running", "phase": "copying_models", "cache": str(cache_tar), "inventory": inv}
        if current_rel:
            payload["current_file"] = current_rel
        _write_status(payload)
        return inv

    emit()
    downloaded_hf: list[str] = []
    for item in manifest_models:
        rel_path = item["path"].lstrip("/")
        dest = LOCAL_MODELS / rel_path
        exp_size = int(item.get("size") or 0)
        if dest.is_file() and (exp_size == 0 or dest.stat().st_size == exp_size) and dest.stat().st_size > 0:
            continue
        dest.parent.mkdir(parents=True, exist_ok=True)
        part = dest.with_name(dest.name + ".part")
        part.unlink(missing_ok=True)
        print(f"HF_PULL {rel_path} {round(exp_size / 1024**3, 2)}GiB (16x)", flush=True)
        t0 = time.monotonic()

        def on_prog(b: int, rel: str = rel_path) -> None:
            progress_map[rel] = b
            emit(rel)

        _download_hf_parallel(item["url"], item.get("headers") or {}, part, exp_size or None, on_prog)
        if exp_size and part.stat().st_size != exp_size:
            part.unlink(missing_ok=True)
            raise RuntimeError(f"Size mismatch for {rel_path}: got {part.stat().st_size}, expected {exp_size}")
        if item.get("sha256"):
            with open(part, "rb") as f:
                digest = hashlib.file_digest(f, "sha256").hexdigest()
            if digest != item["sha256"].lower():
                part.unlink(missing_ok=True)
                raise RuntimeError(f"SHA256 mismatch for {rel_path}")
        os.replace(part, dest)
        progress_map.pop(rel_path, None)
        dt_s = max(0.1, time.monotonic() - t0)
        mb_s = (dest.stat().st_size / 1024**2) / dt_s
        print(f"HF_DONE {rel_path} in {dt_s:.1f}s ({mb_s:.1f} MiB/s)", flush=True)
        downloaded_hf.append(rel_path)
        emit()

    copied_drive: list[str] = []
    for rel_str, src_size in extra_drive.items():
        src = DRIVE_ROOT / "models" / rel_str
        dest = LOCAL_MODELS / rel_str
        if dest.is_file() and dest.stat().st_size == src_size and src_size > 0:
            continue
        dest.parent.mkdir(parents=True, exist_ok=True)
        part = dest.with_name(dest.name + ".part")
        print(f"DRIVE_EXTRA_COPY {rel_str} {round(src_size / 1024**3, 2)}GiB", flush=True)
        sfd = os.open(src, os.O_RDONLY)
        dfd = os.open(part, os.O_CREAT | os.O_TRUNC | os.O_RDWR, 0o644)
        try:
            os.ftruncate(dfd, src_size)
            step = 16 * 1024 * 1024
            offsets = list(range(0, src_size, step))
            lock = threading.Lock()
            done_b = 0
            last_t = 0.0

            def copy_chunk(off: int) -> None:
                nonlocal done_b, last_t
                buf = os.pread(sfd, min(step, src_size - off), off)
                os.pwrite(dfd, buf, off)
                with lock:
                    done_b += len(buf)
                    now = time.monotonic()
                    if now - last_t >= 1.5:
                        last_t = now
                        progress_map[rel_str] = done_b
                        emit(rel_str)

            with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
                for fut in concurrent.futures.as_completed([pool.submit(copy_chunk, o) for o in offsets]):
                    fut.result()
            os.fsync(dfd)
        finally:
            os.close(sfd)
            os.close(dfd)
        os.replace(part, dest)
        progress_map.pop(rel_str, None)
        copied_drive.append(rel_str)
        emit()

    link = APP_DIR / "models"
    if link.is_symlink() or link.is_file():
        link.unlink()
    elif link.exists():
        shutil.rmtree(link)
    link.symlink_to(LOCAL_MODELS)
    return downloaded_hf, copied_drive, _build_inventory_snapshot(manifest_models, extra_drive)


def _stop_comfy() -> None:
    if PID_FILE.is_file():
        try:
            os.kill(int(PID_FILE.read_text().strip()), signal.SIGTERM)
        except (OSError, ValueError):
            pass
        PID_FILE.unlink(missing_ok=True)
    subprocess.run(["pkill", "-f", "main.py --listen 127.0.0.1 --port 8188"], check=False)
    for _ in range(20):
        if not _is_comfy_listening():
            return
        time.sleep(0.5)


def _is_comfy_listening() -> dict | None:
    try:
        with urllib.request.urlopen("http://127.0.0.1:8188/system_stats", timeout=2) as r:
            return json.load(r)
    except Exception:
        return None


STATUS_FILE = pathlib.Path(os.environ.get("CLOUD_DECK_STATUS_FILE", "/tmp/comfy-bootstrap-status.json"))


def _comfy_launch_spec(vpy: pathlib.Path) -> dict:
    # The real NVIDIA driver must precede CUDA toolkit stubs.
    environment = {
        "LD_LIBRARY_PATH": f"/usr/lib64-nvidia:/usr/local/cuda/lib64:{os.environ.get('LD_LIBRARY_PATH', '')}".rstrip(":"),
    }
    run_env = {**os.environ, **environment}
    cmd = [str(vpy), "main.py", "--listen", "127.0.0.1", "--port", "8188", "--preview-method", "none"]
    if subprocess.run([str(vpy), "-c", "import sageattention"], env=run_env, capture_output=True).returncode == 0:
        cmd.append("--use-sage-attention")
    return {"argv": cmd, "workingDirectory": str(APP_DIR), "environment": environment}




def main() -> None:
    prepare_only = os.environ.get("CLOUD_DECK_PREPARE_ONLY") == "1"
    if prepare_only and not os.environ.get("CLOUD_DECK_PREPARED_FILE"):
        raise RuntimeError("Managed preparation requires CLOUD_DECK_PREPARED_FILE.")
    manifest_path = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path("/tmp/comfy-manifest.json")
    manifest = json.loads(manifest_path.read_text())
    _ensure_drive_dirs(require_mount=prepare_only)

    manifest_models = manifest.get("models", [])
    manifest_paths = {m["path"].lstrip("/") for m in manifest_models}
    py_tag = f"py{sys.version_info.major}{sys.version_info.minor}"
    cache_tar = DRIVE_ROOT / "cache" / f"{manifest['bundle_id']}-{py_tag}.tar.zst"

    try:
        live = _is_comfy_listening()
        if live and prepare_only:
            raise RuntimeError("ComfyUI is already running outside this supervisor; stop it explicitly before transferring ownership.")
        if live and _all_dirs_are_local():
            _install_and_start_sync_daemon()
            inv = _build_inventory_snapshot(manifest_models, _scan_drive_extra_models(manifest_paths))
            done = {
                "status": "already_running",
                "phase": "ready",
                "cache": str(cache_tar),
                "models": "local",
                "inventory": inv,
                "system_stats": live,
            }
            _write_status(done)
            print(json.dumps(done, ensure_ascii=False))
            return
        if live:
            _stop_comfy()

        extra_drive_initial = _scan_drive_extra_models(manifest_paths)
        mode = "cache_hit"
        vpy = VENV_DIR / "bin" / "python"
        if not (APP_DIR.is_dir() and vpy.is_file()):
            print("STAGE extracting runtime archive...", flush=True)
            _write_status({
                "status": "running",
                "phase": "extracting",
                "cache": str(cache_tar),
                "inventory": _build_inventory_snapshot(manifest_models, extra_drive_initial),
            })
            if cache_tar.is_file():
                if shutil.which("zstd") is None:
                    subprocess.run(["apt-get", "update", "-qq"], check=False)
                    subprocess.run(["apt-get", "install", "-y", "-qq", "zstd", "ffmpeg"], check=False)
                _run(["tar", "--zstd", "-xf", str(cache_tar), "-C", "/"])
                chk = subprocess.run([str(vpy), "-c", "import torch, aiohttp, comfyui_frontend_package"], capture_output=True)
                if chk.returncode != 0:
                    mode = "rebuild_after_base_change"
                    _build_fresh(manifest, cache_tar)
            else:
                mode = "cold_build"
                _build_fresh(manifest, cache_tar)
        else:
            mode = "warm_vm"

        print("STAGE staging user workflows & input to local disk...", flush=True)
        _stage_local_userdata(preserve_local=prepare_only)
        if not prepare_only:
            _install_and_start_sync_daemon()

        print("STAGE pulling models from Hugging Face (16x) to local NVMe...", flush=True)
        downloaded_models, copied_models, final_inv = _stage_all_models_local(manifest_models, cache_tar)

        if prepare_only:
            prepared_path = os.environ.get("CLOUD_DECK_PREPARED_FILE")
            if not prepared_path:
                raise RuntimeError("Managed preparation requires CLOUD_DECK_PREPARED_FILE.")
            prepared = pathlib.Path(prepared_path)
            prepared.parent.mkdir(parents=True, exist_ok=True)
            temporary = prepared.with_suffix(".tmp")
            temporary.write_text(json.dumps(_comfy_launch_spec(vpy), ensure_ascii=False))
            os.chmod(temporary, 0o600)
            os.replace(temporary, prepared)
            done = {"status": "prepared", "phase": "prepared", "mode": mode, "inventory": final_inv}
            _write_status(done)
            print(json.dumps(done, ensure_ascii=False))
            return

        # Ensure sshd allows concurrent tunnel + interactive shells (Colab defaults to MaxStartups 1)
        sshd_cfg = pathlib.Path("/etc/ssh/sshd_config")
        if sshd_cfg.is_file():
            cfg_txt = sshd_cfg.read_text(errors="replace")
            if "MaxStartups 1\n" in cfg_txt or cfg_txt.strip().endswith("MaxStartups 1"):
                sshd_cfg.write_text(cfg_txt.replace("MaxStartups 1", "MaxStartups 10\nMaxSessions 10"))
                subprocess.run(["pkill", "-HUP", "-x", "sshd"], check=False)

        print("STAGE starting ComfyUI...", flush=True)
        _write_status({"status": "running", "phase": "starting_comfy", "cache": str(cache_tar), "inventory": final_inv})
        # Keep the same launch options for the legacy client and the managed recipe.
        launch = _comfy_launch_spec(vpy)
        run_env = {**os.environ, **launch["environment"]}
        cmd = launch["argv"]

        with open(LOG_FILE, "ab") as lf:
            proc = subprocess.Popen(cmd, cwd=str(APP_DIR), env=run_env, stdin=subprocess.DEVNULL, stdout=lf, stderr=lf, start_new_session=True)
        PID_FILE.write_text(f"{proc.pid}\n")

        deadline = time.time() + 120
        while time.time() < deadline:
            stats = _is_comfy_listening()
            if stats:
                done = {
                    "status": "ready",
                    "phase": "ready",
                    "mode": mode,
                    "pid": proc.pid,
                    "cache": str(cache_tar),
                    "downloaded_models": downloaded_models,
                    "models_copied_local": copied_models,
                    "models": "local",
                    "inventory": _build_inventory_snapshot(manifest_models, _scan_drive_extra_models(manifest_paths)),
                    "system_stats": stats,
                }
                _write_status(done)
                print(json.dumps(done, ensure_ascii=False))
                return
            if proc.poll() is not None:
                tail = LOG_FILE.read_text(errors="replace")[-3000:] if LOG_FILE.is_file() else ""
                raise RuntimeError(f"ComfyUI exited early ({proc.returncode}):\n{tail}")
            time.sleep(1.5)

        tail = LOG_FILE.read_text(errors="replace")[-3000:] if LOG_FILE.is_file() else ""
        raise RuntimeError(f"ComfyUI startup timed out after 120s:\n{tail}")
    except Exception as exc:
        _write_status({"status": "error", "phase": "error", "error": str(exc)})
        raise


if __name__ == "__main__":
    main()
