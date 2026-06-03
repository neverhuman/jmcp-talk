#!/usr/bin/env bash
# One-time setup for the Qwen2.5-Omni-7B-AWQ benchmark/serving venv.
#
# Why a SEPARATE venv from .venv-llm: the Omni audio path needs the transformers
# "v4.51.3-Qwen2.5-Omni-preview" tag, which collides with vLLM 0.9.2's pinned
# transformers 4.53.3. AWQ is mandatory — bf16 Omni-7B + Talker is ~31GB and does
# not fit the 24GB 3090; AWQ + Talker is ~12-18GB for normal turns.
#
# Deps + the audio-system-prompt requirement come from the official AWQ model card.
# Weights download to the HF cache (outside the repo) and are never committed.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENV="$HERE/.venv-omni"
MODEL="${OMNI_MODEL:-Qwen/Qwen2.5-Omni-7B-AWQ}"
PIP="$VENV/bin/pip"
PY="$VENV/bin/python"

step() { echo; echo "=== [omni-setup] $* ==="; }

step "create venv"
python3 -m venv "$VENV"
"$PIP" install -q -U pip wheel

# torch first, cu126 to match the 570/CUDA-12.8 driver (same build .venv-llm uses).
step "install torch cu126"
"$PIP" install -q torch --index-url https://download.pytorch.org/whl/cu126
"$PY" -c "import torch;print('torch',torch.__version__,'cuda?',torch.cuda.is_available())"

# autoawq provides the 4-bit inference kernels. --no-deps so it can't drag torch/
# transformers to versions that break the Omni preview; its pure-python needs
# (accelerate/transformers) are installed explicitly below.
step "install autoawq (no-deps)"
"$PIP" install -q --no-deps autoawq==0.2.9 || echo "[omni-setup] autoawq install issue (continuing)"

step "install transformers Omni-preview + runtime deps"
"$PIP" install -q "git+https://github.com/huggingface/transformers@v4.51.3-Qwen2.5-Omni-preview"
"$PIP" install -q accelerate qwen-omni-utils soundfile huggingface_hub
"$PY" -c "import torch,transformers;print('torch',torch.__version__,'cuda?',torch.cuda.is_available(),'tf',transformers.__version__)"

step "download $MODEL weights to HF cache"
"$PY" - <<PYEOF
from huggingface_hub import snapshot_download
p = snapshot_download("${MODEL}")
print("[omni-setup] weights at", p)
PYEOF

echo
echo "OMNI_SETUP_DONE"
