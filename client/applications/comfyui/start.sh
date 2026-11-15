#!/usr/bin/env bash
set -euo pipefail

# Put the real NVIDIA driver ahead of CUDA toolkit stubs.
export LD_LIBRARY_PATH="/usr/lib64-nvidia:/usr/local/cuda/lib64${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
cd /opt/ComfyUI
arguments=(main.py --listen 127.0.0.1 --port 8188 --preview-method none)
if /opt/comfy-venv/bin/python -c 'import sageattention' >/dev/null 2>&1; then
  arguments+=(--use-sage-attention)
fi
exec /opt/comfy-venv/bin/python "${arguments[@]}"
