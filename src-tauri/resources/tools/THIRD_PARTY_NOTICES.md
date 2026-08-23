# Third-party media components

VideoNotes invokes these components as separate executables. They are not linked into the Rust application binary.

## yt-dlp

- Project: https://github.com/yt-dlp/yt-dlp
- Binary: official Windows x64 release (`yt-dlp.exe`)
- License: Unlicense / public-domain dedication
- Included license: `licenses/yt-dlp-LICENSE.txt`

## FFmpeg and ffprobe

- Project: https://ffmpeg.org/
- Windows build: Gyan Doshi release essentials, https://www.gyan.dev/ffmpeg/builds/
- Build license: GPLv3, as declared by the build distributor
- Included license: `licenses/GPL-3.0.txt`
- Build configuration and component versions: `licenses/FFmpeg-build-README.txt`

## whisper.cpp

- Project: https://github.com/ggml-org/whisper.cpp
- Binary: official Windows x64 CPU release (`whisper/whisper-cli.exe` and its isolated runtime DLLs)
- License: MIT
- Included license: `licenses/whisper.cpp-LICENSE.txt`

## llama.cpp

- Project: https://github.com/ggml-org/llama.cpp
- Binary: official Windows x64 CPU release (`llama/llama-cli.exe` and runtime DLLs)
- License: MIT
- Included license: `licenses/llama.cpp-LICENSE.txt`

The exact download URLs and SHA-256 values used for a local build are recorded in `media-tools.lock.json`, `whisper/whisper-worker.lock.json`, and `llama/llama-worker.lock.json`. Before publicly distributing an installer, complete the applicable GPL source-code and notice obligations for the exact FFmpeg build being shipped.
