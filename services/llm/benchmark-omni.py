#!/usr/bin/env python3
"""Decision-grade latency benchmark for Qwen2.5-Omni-7B-AWQ on a single GPU.

Measures the full speech-to-speech turn: spoken-audio IN -> (Thinker text +
Talker speech) OUT, the number that decides whether native local omni can be
"OpenAI-fast" on a 3090. `model.generate(return_audio=True)` returns the whole
reply at once, so this times the TOTAL turn (not streamed first-audio) — but a
short-turn total of ~1-2s implies sub-second streamed first-audio, while ~10-20s
means streaming can't save it.

Usage: python benchmark-omni.py <input_question.wav>
Run inside services/llm/.venv-omni. Requires the GPU free (stop the 30B first).
"""
import sys
import time

import soundfile as sf
import torch
import torch.nn as nn
from transformers import Qwen2_5OmniForConditionalGeneration, Qwen2_5OmniProcessor
from qwen_omni_utils import process_mm_info

# --- AWQ-omni load fix -------------------------------------------------------
# The published checkpoint only lists 'visual' in modules_to_not_convert, so
# transformers tries to AWQ-wrap every other Linear — including a few Talker/
# token2wav layers whose in/out features are NOT divisible by group_size (128).
# Those were left fp16 in the checkpoint precisely because GEMM-AWQ requires
# 128-divisibility; wrapping them trips `assert in_features % group_size == 0`.
# Pre-scan and add such layers' leaf names to the skip-list so they load as
# plain fp16 Linears (their real weights are in the checkpoint).
import transformers.integrations.awq as _awq

_orig_replace = _awq.replace_with_awq_linear


def _replace_skipping_indivisible(model, modules_to_not_convert=None, quantization_config=None, **kw):
    skip = list(modules_to_not_convert or [])
    gs = quantization_config.group_size
    for full, mod in model.named_modules():
        if isinstance(mod, nn.Linear) and (mod.in_features % gs or mod.out_features % gs):
            leaf = full.split(".")[-1]
            if leaf not in skip:
                skip.append(leaf)
    return _orig_replace(model, modules_to_not_convert=skip,
                         quantization_config=quantization_config, **kw)


_awq.replace_with_awq_linear = _replace_skipping_indivisible
# -----------------------------------------------------------------------------

MODEL = "Qwen/Qwen2.5-Omni-7B-AWQ"
# The AWQ card requires this exact system prompt or speech output may not emit.
SYS = (
    "You are Qwen, a virtual human developed by the Qwen Team, Alibaba Group, "
    "capable of perceiving auditory and visual inputs, as well as generating text and speech."
)


def main() -> None:
    if len(sys.argv) < 2:
        print("usage: benchmark-omni.py <input_question.wav>")
        raise SystemExit(2)
    wav = sys.argv[1]

    t0 = time.perf_counter()
    # sdpa attention (flash-attn not installed); the dominant cost is the
    # autoregressive Talker + code2wav, not the attention kernel.
    # float16 (NOT bf16/"auto"): the autoawq GEMM dequant kernel only supports
    # fp16 — bf16 trips "expected scalar type Int but found BFloat16".
    # device_map=None then .to("cuda"): accelerate's device_map dispatch casts
    # the int32 qweight buffers to fp16 (-> "expected Int but found Half"); a
    # plain device move preserves integer dtypes, so load on CPU and move.
    model = Qwen2_5OmniForConditionalGeneration.from_pretrained(
        MODEL, torch_dtype=torch.float16, device_map=None, attn_implementation="sdpa"
    )
    model = model.to("cuda").eval()
    proc = Qwen2_5OmniProcessor.from_pretrained(MODEL)
    load_s = time.perf_counter() - t0
    print(f"[bench] model+processor load: {load_s:.1f}s")
    print(f"[bench] VRAM after load: {torch.cuda.memory_allocated()/1e9:.1f} GB allocated, "
          f"{torch.cuda.max_memory_allocated()/1e9:.1f} GB peak")

    info = sf.info(wav)
    in_dur = info.frames / info.samplerate
    print(f"[bench] input question: {wav} ({in_dur:.1f}s of speech)")

    conv = [
        {"role": "system", "content": [{"type": "text", "text": SYS}]},
        {"role": "user", "content": [{"type": "audio", "audio": wav}]},
    ]

    def one_turn(tag: str) -> None:
        torch.cuda.synchronize()
        t = time.perf_counter()
        text = proc.apply_chat_template(conv, add_generation_prompt=True, tokenize=False)
        audios, images, videos = process_mm_info(conv, use_audio_in_video=False)
        inputs = proc(
            text=text, audio=audios, images=images, videos=videos,
            return_tensors="pt", padding=True, use_audio_in_video=False,
        ).to(model.device).to(model.dtype)
        prep_s = time.perf_counter() - t

        t = time.perf_counter()
        text_ids, audio = model.generate(
            **inputs, use_audio_in_video=False, return_audio=True,
            speaker="Chelsie", max_new_tokens=128,
        )
        torch.cuda.synchronize()
        gen_s = time.perf_counter() - t

        out = audio.reshape(-1).detach().cpu().numpy()
        out_dur = len(out) / 24000.0
        reply = proc.batch_decode(text_ids, skip_special_tokens=True)[0]
        sf.write(f"/tmp/omni-reply-{tag}.wav", out, 24000)
        rtf = gen_s / out_dur if out_dur > 0 else float("nan")
        print(f"[bench] {tag}: prep {prep_s:.2f}s + generate {gen_s:.2f}s "
              f"=> TURN {prep_s+gen_s:.2f}s | spoke {out_dur:.1f}s (RTF {rtf:.2f})")
        print(f"[bench] {tag} reply text: {reply[-160:]!r}")

    print("[bench] --- cold turn (includes CUDA warmup/JIT) ---")
    one_turn("cold")
    print("[bench] --- warm turns (steady-state latency) ---")
    one_turn("warm1")
    one_turn("warm2")
    print("[bench] DONE")


if __name__ == "__main__":
    main()
