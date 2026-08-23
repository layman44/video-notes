use libloading::Library;
use std::{
    ffi::{c_char, c_void, CStr, CString},
    path::Path,
    ptr,
};

#[derive(Debug, Clone)]
pub struct CtcWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

#[repr(C)]
struct SherpaOnnxFeatureConfig {
    sample_rate: i32,
    feature_dim: i32,
}
#[repr(C)]
struct SherpaOnnxOfflineTransducerModelConfig { encoder: *const c_char, decoder: *const c_char, joiner: *const c_char }
#[repr(C)]
struct SherpaOnnxOfflineParaformerModelConfig { model: *const c_char }
#[repr(C)]
struct SherpaOnnxOfflineNemoEncDecCtcModelConfig { model: *const c_char }
#[repr(C)]
struct SherpaOnnxOfflineWhisperModelConfig {
    encoder: *const c_char, decoder: *const c_char, language: *const c_char, task: *const c_char,
    tail_paddings: i32, enable_token_timestamps: i32, enable_segment_timestamps: i32,
}
#[repr(C)]
struct SherpaOnnxOfflineCanaryModelConfig {
    encoder: *const c_char, decoder: *const c_char, src_lang: *const c_char, tgt_lang: *const c_char, use_pnc: i32,
}
#[repr(C)]
struct SherpaOnnxOfflineCohereTranscribeModelConfig {
    encoder: *const c_char, decoder: *const c_char, language: *const c_char, use_punct: i32, use_itn: i32,
}
#[repr(C)]
struct SherpaOnnxOfflineFireRedAsrModelConfig { encoder: *const c_char, decoder: *const c_char }
#[repr(C)]
struct SherpaOnnxOfflineFireRedAsrCtcModelConfig { model: *const c_char }
#[repr(C)]
struct SherpaOnnxOfflineMoonshineModelConfig {
    preprocessor: *const c_char, encoder: *const c_char, uncached_decoder: *const c_char,
    cached_decoder: *const c_char, merged_decoder: *const c_char,
}
#[repr(C)]
struct SherpaOnnxOfflineTdnnModelConfig { model: *const c_char }
#[repr(C)]
struct SherpaOnnxOfflineLMConfig { model: *const c_char, scale: f32 }
#[repr(C)]
struct SherpaOnnxOfflineSenseVoiceModelConfig { model: *const c_char, language: *const c_char, use_itn: i32 }
#[repr(C)]
struct SherpaOnnxOfflineDolphinModelConfig { model: *const c_char }
#[repr(C)]
struct SherpaOnnxOfflineZipformerCtcModelConfig { model: *const c_char }
#[repr(C)]
struct SherpaOnnxOfflineWenetCtcModelConfig { model: *const c_char }
#[repr(C)]
struct SherpaOnnxOfflineOmnilingualAsrCtcModelConfig { model: *const c_char }
#[repr(C)]
struct SherpaOnnxOfflineFunASRNanoModelConfig {
    encoder_adaptor: *const c_char, llm: *const c_char, embedding: *const c_char, tokenizer: *const c_char,
    system_prompt: *const c_char, user_prompt: *const c_char, max_new_tokens: i32, temperature: f32,
    top_p: f32, seed: i32, language: *const c_char, itn: i32, hotwords: *const c_char,
}
#[repr(C)]
struct SherpaOnnxOfflineQwen3ASRModelConfig {
    conv_frontend: *const c_char, encoder: *const c_char, decoder: *const c_char, tokenizer: *const c_char,
    max_total_len: i32, max_new_tokens: i32, temperature: f32, top_p: f32, seed: i32, hotwords: *const c_char,
}
#[repr(C)]
struct SherpaOnnxOfflineMedAsrCtcModelConfig { model: *const c_char }
#[repr(C)]
struct SherpaOnnxOfflineModelConfig {
    transducer: SherpaOnnxOfflineTransducerModelConfig,
    paraformer: SherpaOnnxOfflineParaformerModelConfig,
    nemo_ctc: SherpaOnnxOfflineNemoEncDecCtcModelConfig,
    whisper: SherpaOnnxOfflineWhisperModelConfig,
    tdnn: SherpaOnnxOfflineTdnnModelConfig,
    tokens: *const c_char,
    num_threads: i32,
    debug: i32,
    provider: *const c_char,
    model_type: *const c_char,
    modeling_unit: *const c_char,
    bpe_vocab: *const c_char,
    telespeech_ctc: *const c_char,
    sense_voice: SherpaOnnxOfflineSenseVoiceModelConfig,
    moonshine: SherpaOnnxOfflineMoonshineModelConfig,
    fire_red_asr: SherpaOnnxOfflineFireRedAsrModelConfig,
    dolphin: SherpaOnnxOfflineDolphinModelConfig,
    zipformer_ctc: SherpaOnnxOfflineZipformerCtcModelConfig,
    canary: SherpaOnnxOfflineCanaryModelConfig,
    wenet_ctc: SherpaOnnxOfflineWenetCtcModelConfig,
    omnilingual: SherpaOnnxOfflineOmnilingualAsrCtcModelConfig,
    medasr: SherpaOnnxOfflineMedAsrCtcModelConfig,
    funasr_nano: SherpaOnnxOfflineFunASRNanoModelConfig,
    fire_red_asr_ctc: SherpaOnnxOfflineFireRedAsrCtcModelConfig,
    qwen3_asr: SherpaOnnxOfflineQwen3ASRModelConfig,
    cohere_transcribe: SherpaOnnxOfflineCohereTranscribeModelConfig,
}
#[repr(C)]
struct SherpaOnnxHomophoneReplacerConfig { dict_dir: *const c_char, lexicon: *const c_char, rule_fsts: *const c_char }
#[repr(C)]
struct SherpaOnnxOfflineRecognizerConfig {
    feat_config: SherpaOnnxFeatureConfig,
    model_config: SherpaOnnxOfflineModelConfig,
    lm_config: SherpaOnnxOfflineLMConfig,
    decoding_method: *const c_char,
    max_active_paths: i32,
    hotwords_file: *const c_char,
    hotwords_score: f32,
    rule_fsts: *const c_char,
    rule_fars: *const c_char,
    blank_penalty: f32,
    hr: SherpaOnnxHomophoneReplacerConfig,
}
#[repr(C)]
struct SherpaOnnxOfflineRecognizerResult {
    text: *const c_char,
    timestamps: *mut f32,
    count: i32,
    tokens: *const c_char,
    tokens_arr: *const *const c_char,
    json: *const c_char,
    lang: *const c_char,
    emotion: *const c_char,
    event: *const c_char,
    durations: *mut f32,
    ys_log_probs: *mut f32,
    segment_timestamps: *const f32,
    segment_durations: *const f32,
    segment_texts: *const c_char,
    segment_texts_arr: *const *const c_char,
    segment_count: i32,
}

type CreateRecognizerFn = unsafe extern "C" fn(*const SherpaOnnxOfflineRecognizerConfig) -> *const c_void;
type DestroyRecognizerFn = unsafe extern "C" fn(*const c_void);
type CreateStreamFn = unsafe extern "C" fn(*const c_void) -> *const c_void;
type DestroyStreamFn = unsafe extern "C" fn(*const c_void);
type AcceptWaveformFn = unsafe extern "C" fn(*const c_void, i32, *const f32, i32);
type DecodeFn = unsafe extern "C" fn(*const c_void, *const c_void);
type GetResultFn = unsafe extern "C" fn(*const c_void) -> *const SherpaOnnxOfflineRecognizerResult;
type DestroyResultFn = unsafe extern "C" fn(*const SherpaOnnxOfflineRecognizerResult);

#[cfg(windows)]
fn prepare_dll_search_dir(dll_path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" { fn SetDllDirectoryW(lp_path_name: *const u16) -> i32; }
    let parent = dll_path.parent().ok_or_else(|| format!("无法获取 sherpa-onnx DLL 目录：{}", dll_path.display()))?;
    let mut wide: Vec<u16> = parent.as_os_str().encode_wide().collect();
    wide.push(0);
    if unsafe { SetDllDirectoryW(wide.as_ptr()) } == 0 {
        return Err(format!("设置 sherpa-onnx DLL 搜索目录失败：{}", parent.display()));
    }
    Ok(())
}
#[cfg(windows)]
fn reset_dll_search_dir() {
    #[link(name = "kernel32")]
    extern "system" { fn SetDllDirectoryW(lp_path_name: *const u16) -> i32; }
    unsafe { let _ = SetDllDirectoryW(ptr::null()); }
}
#[cfg(not(windows))]
fn prepare_dll_search_dir(_dll_path: &Path) -> Result<(), String> { Ok(()) }
#[cfg(not(windows))]
fn reset_dll_search_dir() {}

pub struct CtcRecognizer {
    _library: Library,
    recognizer: *const c_void,
    destroy_recognizer: DestroyRecognizerFn,
    create_stream: CreateStreamFn,
    destroy_stream: DestroyStreamFn,
    accept_waveform: AcceptWaveformFn,
    decode: DecodeFn,
    get_result: GetResultFn,
    destroy_result: DestroyResultFn,
}

impl Drop for CtcRecognizer {
    fn drop(&mut self) {
        if !self.recognizer.is_null() {
            unsafe { (self.destroy_recognizer)(self.recognizer) };
        }
    }
}

impl CtcRecognizer {
    pub fn new(dll_path: &Path, model_path: &Path, tokens_path: &Path, threads: usize) -> Result<Self, String> {
        if !dll_path.is_file() { return Err(format!("sherpa-onnx C API DLL 不存在：{}", dll_path.display())); }
        if !model_path.is_file() { return Err(format!("English CTC INT8 模型不存在：{}", model_path.display())); }
        if !tokens_path.is_file() { return Err(format!("English CTC tokens.txt 不存在：{}", tokens_path.display())); }

        prepare_dll_search_dir(dll_path)?;
        let lib_result = unsafe { Library::new(dll_path) };
        reset_dll_search_dir();
        let library = lib_result.map_err(|e| format!("加载 sherpa-onnx C API DLL 失败：{}：{e}", dll_path.display()))?;

        unsafe {
            let create_recognizer: CreateRecognizerFn = *library.get::<CreateRecognizerFn>(b"SherpaOnnxCreateOfflineRecognizer\0")
                .map_err(|e| format!("找不到 SherpaOnnxCreateOfflineRecognizer：{e}"))?;
            let destroy_recognizer: DestroyRecognizerFn = *library.get::<DestroyRecognizerFn>(b"SherpaOnnxDestroyOfflineRecognizer\0")
                .map_err(|e| format!("找不到 SherpaOnnxDestroyOfflineRecognizer：{e}"))?;
            let create_stream: CreateStreamFn = *library.get::<CreateStreamFn>(b"SherpaOnnxCreateOfflineStream\0")
                .map_err(|e| format!("找不到 SherpaOnnxCreateOfflineStream：{e}"))?;
            let destroy_stream: DestroyStreamFn = *library.get::<DestroyStreamFn>(b"SherpaOnnxDestroyOfflineStream\0")
                .map_err(|e| format!("找不到 SherpaOnnxDestroyOfflineStream：{e}"))?;
            let accept_waveform: AcceptWaveformFn = *library.get::<AcceptWaveformFn>(b"SherpaOnnxAcceptWaveformOffline\0")
                .map_err(|e| format!("找不到 SherpaOnnxAcceptWaveformOffline：{e}"))?;
            let decode: DecodeFn = *library.get::<DecodeFn>(b"SherpaOnnxDecodeOfflineStream\0")
                .map_err(|e| format!("找不到 SherpaOnnxDecodeOfflineStream：{e}"))?;
            let get_result: GetResultFn = *library.get::<GetResultFn>(b"SherpaOnnxGetOfflineStreamResult\0")
                .map_err(|e| format!("找不到 SherpaOnnxGetOfflineStreamResult：{e}"))?;
            let destroy_result: DestroyResultFn = *library.get::<DestroyResultFn>(b"SherpaOnnxDestroyOfflineRecognizerResult\0")
                .map_err(|e| format!("找不到 SherpaOnnxDestroyOfflineRecognizerResult：{e}"))?;

            let model = CString::new(model_path.to_string_lossy().as_bytes()).map_err(|_| "CTC 模型路径包含 NUL 字节".to_string())?;
            let tokens = CString::new(tokens_path.to_string_lossy().as_bytes()).map_err(|_| "CTC tokens 路径包含 NUL 字节".to_string())?;
            let provider = CString::new("cpu").unwrap();
            let decoding = CString::new("greedy_search").unwrap();
            let mut config: SherpaOnnxOfflineRecognizerConfig = std::mem::zeroed();
            config.feat_config.sample_rate = 16000;
            config.feat_config.feature_dim = 80;
            config.model_config.nemo_ctc.model = model.as_ptr();
            config.model_config.tokens = tokens.as_ptr();
            config.model_config.num_threads = threads.clamp(1, 4) as i32;
            config.model_config.provider = provider.as_ptr();
            config.decoding_method = decoding.as_ptr();
            let recognizer = create_recognizer(&config);
            if recognizer.is_null() {
                return Err("创建 sherpa-onnx English CTC recognizer 失败，请检查 v1.13.6 DLL、model.int8.onnx 与 tokens.txt 是否匹配".into());
            }

            Ok(Self {
                _library: library,
                recognizer,
                destroy_recognizer,
                create_stream,
                destroy_stream,
                accept_waveform,
                decode,
                get_result,
                destroy_result,
            })
        }
    }

    pub fn decode_pcm(&self, samples: &[f32], sample_rate: i32) -> Result<Vec<CtcWord>, String> {
        if samples.is_empty() { return Ok(Vec::new()); }
        if samples.len() > i32::MAX as usize { return Err("CTC 局部音频样本过长".into()); }
        unsafe {
            let stream = (self.create_stream)(self.recognizer);
            if stream.is_null() { return Err("创建 sherpa-onnx offline stream 失败".into()); }
            struct StreamGuard { ptr: *const c_void, destroy: DestroyStreamFn }
            impl Drop for StreamGuard { fn drop(&mut self) { unsafe { (self.destroy)(self.ptr) } } }
            let _stream_guard = StreamGuard { ptr: stream, destroy: self.destroy_stream };

            (self.accept_waveform)(stream, sample_rate, samples.as_ptr(), samples.len() as i32);
            (self.decode)(self.recognizer, stream);
            let result = (self.get_result)(stream);
            if result.is_null() { return Err("sherpa-onnx CTC 返回空 result".into()); }
            struct ResultGuard { ptr: *const SherpaOnnxOfflineRecognizerResult, destroy: DestroyResultFn }
            impl Drop for ResultGuard { fn drop(&mut self) { unsafe { (self.destroy)(self.ptr) } } }
            let _result_guard = ResultGuard { ptr: result, destroy: self.destroy_result };
            let r = &*result;
            if r.count <= 0 || r.tokens_arr.is_null() || r.timestamps.is_null() {
                let text = if r.text.is_null() { String::new() } else { CStr::from_ptr(r.text).to_string_lossy().into_owned() };
                return Err(format!("English CTC 没有返回 token timestamps；text={}", text.trim()));
            }
            let count = r.count as usize;
            let token_ptrs = std::slice::from_raw_parts(r.tokens_arr, count);
            let timestamps = std::slice::from_raw_parts(r.timestamps, count);
            let durations = if r.durations.is_null() { None } else { Some(std::slice::from_raw_parts(r.durations, count)) };
            let mut tokens = Vec::with_capacity(count);
            for i in 0..count {
                if token_ptrs[i].is_null() { continue; }
                let text = CStr::from_ptr(token_ptrs[i]).to_string_lossy().into_owned();
                let start = timestamps[i] as f64;
                let duration = durations.map(|d| d[i] as f64).unwrap_or(0.0).max(0.0);
                tokens.push((text, start, duration));
            }
            Ok(tokens_to_words(&tokens))
        }
    }
}

fn is_blank_token(s: &str) -> bool {
    let t = s.trim();
    matches!(t, "<blk>" | "<blank>" | "<pad>" | "<eps>") || (s.is_empty())
}

fn tokens_to_words(tokens: &[(String, f64, f64)]) -> Vec<CtcWord> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_start = 0.0;
    let mut current_end = 0.0;

    let flush = |out: &mut Vec<CtcWord>, text: &mut String, start: &mut f64, end: &mut f64| {
        let cleaned = text.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '\'').to_ascii_lowercase();
        if !cleaned.is_empty() {
            out.push(CtcWord { text: cleaned, start: *start, end: (*end).max(*start + 0.02) });
        }
        text.clear();
        *start = 0.0;
        *end = 0.0;
    };

    // NeMo CTC exposes token *start* timestamps; durations are generally empty
    // (the C API durations field is mainly useful for TDT-style models). Do not
    // use the next token's start as this token's end: that would collapse every
    // inter-word silence to zero. A small frame-span estimate preserves the
    // relative pause size, which is all this selective repair needs.
    const DEFAULT_CTC_TOKEN_SPAN: f64 = 0.08;
    for (raw, start, duration) in tokens.iter() {
        if is_blank_token(raw) { continue; }
        let token_end = if *duration > 0.001 { *start + *duration } else { *start + DEFAULT_CTC_TOKEN_SPAN };
        let normalized = raw.replace('▁', " ").replace('Ġ', " ");
        let mut chars = normalized.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch.is_whitespace() {
                flush(&mut out, &mut current, &mut current_start, &mut current_end);
                continue;
            }
            if ch.is_ascii_alphabetic() || ch.is_ascii_digit() || ch == '\'' {
                if current.is_empty() { current_start = *start; }
                current.push(ch.to_ascii_lowercase());
                current_end = token_end;
            }
        }
    }
    flush(&mut out, &mut current, &mut current_start, &mut current_end);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn char_tokens_make_words() {
        let t = vec![
            ("y".into(), 0.1, 0.05), ("e".into(), 0.2, 0.05), ("t".into(), 0.3, 0.05),
            (" ".into(), 0.4, 0.05), ("s".into(), 0.5, 0.05), ("h".into(), 0.6, 0.05), ("e".into(), 0.7, 0.05),
        ];
        let w = tokens_to_words(&t);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].text, "yet");
        assert_eq!(w[1].text, "she");
    }
}
