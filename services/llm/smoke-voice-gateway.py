#!/usr/bin/env python3
"""Compatibility wrapper for the direct real TTS smoke analyzer."""

from __future__ import annotations

import runpy
from pathlib import Path


if __name__ == "__main__":
    runpy.run_path(str(Path(__file__).with_name("smoke-real-tts.py")), run_name="__main__")
