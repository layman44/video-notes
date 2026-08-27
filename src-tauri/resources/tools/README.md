# Media tools

Run both commands from the project root before creating a release installer:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/fetch-media-tools.ps1
powershell -ExecutionPolicy Bypass -File scripts/fetch-llama-worker.ps1
powershell -ExecutionPolicy Bypass -File scripts/fetch-openasr-worker.ps1
```

The first script downloads the official `yt-dlp.exe` release and the FFmpeg release essentials Windows build linked from FFmpeg's download page. The second script downloads pinned official llama.cpp Windows x64 CPU workers. Every archive is verified against a fixed SHA-256 value. All CPU backend variants are retained so the worker can select the best instruction set supported by the user's processor.

The OpenASR script downloads the pinned 0.1.30 Windows x64 CPU worker. These executables and their licenses are bundled with the Windows installer. Model files remain separate and are downloaded by the app after installation. MOSS transcription runs with `OPENASR_OFFLINE=1` and does not require an authorization token.
