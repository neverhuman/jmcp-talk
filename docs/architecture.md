# JMCP Talk Architecture

`jmcp-talk` owns speech runtime adapters, deterministic speech fixtures, local
ASR/TTS sidecars, and opt-in MiniCPM-o live validation.

Core authority remains in `jmcp-core`. Talk components route durable approvals,
tool policy, ledgers, and turn state through core APIs.
