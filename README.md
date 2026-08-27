# VideoNotes

VideoNotes 是一款 Windows 本地优先桌面应用：输入公开视频链接，在本机提取音频、完成语音识别和内容整理，并导出带时间戳的 Markdown 笔记。

当前状态：本地媒体处理、多语言高精度语音识别（Fun-ASR-Nano）、语义向量检索（Qwen3-Embedding）、非中文转录翻译、内容整理和 Markdown 导出均已接入。应用使用 Fun-ASR-Nano 在 CPU 上生成带时间戳的原始转录，并复用本地 Qwen3.5 将非中文内容翻译为简体中文、生成结构化笔记。

## 已确定的产品边界

- Windows x64 桌面应用，以 NSIS `Setup.exe` 分发。
- 目标设备为 16GB 内存、无独立显卡、支持 AVX2 的现代 6 核 CPU。
- 首版支持抖音、哔哩哔哩的公开且无需登录的视频链接。
- 不读取或依赖平台字幕，所有内容统一从音频转写。
- 不做 OCR，不分析画面、PPT、代码或画面内文字。
- 语音识别、语义向量检索和 Markdown 整理默认均在本地完成。
- 模型不包含在安装包内，首次启动后按需下载。
- 技术方向：Rust、Tauri 2、React/TypeScript、Fun-ASR-Nano (SenseVoice + CTC + VAD)、Qwen3-Embedding (ONNX)、llama.cpp (Qwen3.5 2B)、yt-dlp、FFmpeg、SQLite。

## 当前已实现

- 严格限制为抖音、哔哩哔哩公开链接，拒绝仿冒域名、本机地址和文件协议。
- 真实媒体元数据解析，不读取平台字幕。
- yt-dlp 音频优先下载、`.part` 续传和机器可解析进度。
- FFmpeg 流式转换为 16kHz、单声道、PCM 16-bit WAV，并按约 30 分钟切片。
- 任务级本地目录、媒体清单、结果复用、进度事件、取消和失败重试。
- 应用内下载 Fun-ASR-Nano 多语言语音模型、Qwen3-Embedding 语义大模型与 Qwen3.5 结构化总结模型，优先使用 HF-Mirror 与魔搭国内镜像、失败后自动切换 Hugging Face 官方源，并支持跨下载源断点续传、SHA-256 完整性校验和模型删除。
- 纯 CPU 顺序转写音频切片，自动检测语种，限制线程数并降低进程优先级以控制资源占用。
- 切片级 JSON 缓存与断点恢复，聚合保存带时间戳的 `transcript.json` 和纯文本 `transcript.txt`。
- 基于 Qwen3-Embedding 的 1024 维本地混合语义搜索（Vector + FTS5 BM25 + RRF 融合排序）。
- 非中文转录使用本地大模型分批翻译为简体中文，每批结果即时保存；笔记整理优先使用中文译文。
- 任务详情页提供中文、双语和原文三种转录显示方式，并同时搜索原文与译文。
- 导出带摘要、要点、章节、中文译文和原文的真实 Markdown 笔记。
- 设置页媒体组件健康检查。
- SQLite 任务状态与业务错误持久化。

尚未实现：针对低置信度片段的自动二次复核与纠错。

## 文档

- [产品需求文档](docs/PRD.md)
- [视觉规范](docs/design/DESIGN_SYSTEM.md)

## 本地开发

当前验证环境：Node.js 22、pnpm 11、Rust 1.96，以及 Tauri 2 在 Windows 上要求的 WebView2 和 MSVC 构建工具。

```powershell
pnpm install
pnpm tools:fetch
pnpm dev
```

启动桌面窗口：

```powershell
pnpm tauri dev
```

生成 NSIS `Setup.exe`：

```powershell
pnpm tauri build
```

模型文件不进入安装包，由应用内的模型管理器按需下载到用户数据目录。

`pnpm tools:fetch` 会准备 yt-dlp 与 FFmpeg。第三方说明见 [THIRD_PARTY_NOTICES.md](src-tauri/resources/tools/THIRD_PARTY_NOTICES.md)。公开分发安装包前，需要完成对应 FFmpeg GPLv3 构建的源代码与通知合规检查。
