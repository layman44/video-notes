# OpenASR Runtime

Run `powershell -ExecutionPolicy Bypass -File scripts/fetch-openasr-worker.ps1` from the repository root to download the pinned Windows x64 OpenASR 0.1.30 runtime into this directory before packaging the desktop application.

The runtime is only the local CPU server. The MOSS q4 model is downloaded separately from the Models page and is verified before use. Production transcription sets `OPENASR_OFFLINE=1`.
