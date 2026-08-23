use libloading::Library;
use std::{
    ffi::{c_char, c_void, CStr, CString},
    path::Path,
    ptr,
};

#[repr(C)]
struct SherpaOnnxOfflinePunctuationModelConfig {
    ct_transformer: *const c_char,
    num_threads: i32,
    debug: i32,
    provider: *const c_char,
}

#[repr(C)]
struct SherpaOnnxOfflinePunctuationConfig {
    model: SherpaOnnxOfflinePunctuationModelConfig,
}

type CreateFn = unsafe extern "C" fn(*const SherpaOnnxOfflinePunctuationConfig) -> *const c_void;
type AddFn = unsafe extern "C" fn(*const c_void, *const c_char) -> *const c_char;
type FreeTextFn = unsafe extern "C" fn(*const c_char);
type DestroyFn = unsafe extern "C" fn(*const c_void);

#[cfg(windows)]
fn prepare_dll_search_dir(dll_path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetDllDirectoryW(lp_path_name: *const u16) -> i32;
    }

    let parent = dll_path
        .parent()
        .ok_or_else(|| format!("无法获取 sherpa-onnx DLL 目录：{}", dll_path.display()))?;
    let mut wide: Vec<u16> = parent.as_os_str().encode_wide().collect();
    wide.push(0);
    let ok = unsafe { SetDllDirectoryW(wide.as_ptr()) };
    if ok == 0 {
        return Err(format!("设置 sherpa-onnx DLL 搜索目录失败：{}", parent.display()));
    }
    Ok(())
}

#[cfg(windows)]
fn reset_dll_search_dir() {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetDllDirectoryW(lp_path_name: *const u16) -> i32;
    }
    unsafe {
        let _ = SetDllDirectoryW(ptr::null());
    }
}

#[cfg(not(windows))]
fn prepare_dll_search_dir(_dll_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
fn reset_dll_search_dir() {}

pub fn punctuate_batch(
    dll_path: &Path,
    model_path: &Path,
    texts: &[String],
) -> Result<Vec<String>, String> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    if !dll_path.is_file() {
        return Err(format!("sherpa-onnx C API DLL 不存在：{}", dll_path.display()));
    }
    if !model_path.is_file() {
        return Err(format!("标点模型不存在：{}", model_path.display()));
    }

    prepare_dll_search_dir(dll_path)?;
    let lib_result = unsafe { Library::new(dll_path) };
    reset_dll_search_dir();
    let lib = lib_result.map_err(|e| {
        format!(
            "加载 sherpa-onnx C API DLL 失败：{}：{e}。请确认同目录依赖 DLL 完整。",
            dll_path.display()
        )
    })?;

    unsafe {
        let create: libloading::Symbol<CreateFn> = lib
            .get(b"SherpaOnnxCreateOfflinePunctuation\0")
            .map_err(|e| format!("找不到 SherpaOnnxCreateOfflinePunctuation：{e}"))?;
        let add: libloading::Symbol<AddFn> = lib
            .get(b"SherpaOfflinePunctuationAddPunct\0")
            .map_err(|e| format!("找不到 SherpaOfflinePunctuationAddPunct：{e}"))?;
        let free_text: libloading::Symbol<FreeTextFn> = lib
            .get(b"SherpaOfflinePunctuationFreeText\0")
            .map_err(|e| format!("找不到 SherpaOfflinePunctuationFreeText：{e}"))?;
        let destroy: libloading::Symbol<DestroyFn> = lib
            .get(b"SherpaOnnxDestroyOfflinePunctuation\0")
            .map_err(|e| format!("找不到 SherpaOnnxDestroyOfflinePunctuation：{e}"))?;

        let model = CString::new(model_path.to_string_lossy().as_bytes())
            .map_err(|_| "标点模型路径包含 NUL 字节".to_string())?;
        let provider = CString::new("cpu").unwrap();
        let config = SherpaOnnxOfflinePunctuationConfig {
            model: SherpaOnnxOfflinePunctuationModelConfig {
                ct_transformer: model.as_ptr(),
                num_threads: 1,
                debug: 0,
                provider: provider.as_ptr(),
            },
        };
        let punct = create(&config);
        if punct.is_null() {
            return Err("创建 sherpa-onnx OfflinePunctuation 失败，请检查模型和 DLL 版本".into());
        }

        struct Guard<'a> {
            ptr: *const c_void,
            destroy: &'a DestroyFn,
        }
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                unsafe { (self.destroy)(self.ptr) }
            }
        }
        let _guard = Guard { ptr: punct, destroy: &*destroy };

        let mut result = Vec::with_capacity(texts.len());
        for text in texts {
            // CString contains the exact UTF-8 bytes produced by Rust. This bypasses
            // Windows narrow argv/ACP conversion that corrupted Chinese in v0.5.1.
            let input = CString::new(text.as_bytes())
                .map_err(|_| "识别文本包含 NUL 字节，无法送入标点模型".to_string())?;
            let output_ptr = add(punct, input.as_ptr());
            if output_ptr.is_null() {
                return Err("sherpa-onnx 标点 C API 返回空指针".into());
            }
            let output = CStr::from_ptr(output_ptr).to_string_lossy().into_owned();
            free_text(output_ptr);
            if output.trim().is_empty() {
                return Err("sherpa-onnx 标点 C API 返回空文本".into());
            }
            result.push(output);
        }
        Ok(result)
    }
}
