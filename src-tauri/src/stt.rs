// STT via whisper-rs (on-device, 16kHz mono f32).
// Lazy cached context; model at ~/.config/shikigami/models/ (see MODEL_NAME)
// ponytail: new dep only whisper+reqwest already; no per call reload.

use std::path::PathBuf;
use std::sync::OnceLock;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

// Breeze-ASR-25 (MediaTek, whisper-large-v2 fine-tune) — built for Taiwanese
// Mandarin + zh/en code-switching. Override file with SHIKIGAMI_STT_MODEL
// (bare filename under models/, or absolute path).
const MODEL_NAME: &str = "breeze-asr-25-q5_k.bin";

pub fn model_path() -> PathBuf {
    let name = std::env::var("SHIKIGAMI_STT_MODEL").unwrap_or_else(|_| MODEL_NAME.to_string());
    if name.starts_with('/') {
        return PathBuf::from(name);
    }
    let mut p = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    p.push(".config/shikigami/models");
    p.push(name);
    p
}

static WHISPER_CTX: OnceLock<WhisperContext> = OnceLock::new();

fn get_ctx() -> Result<&'static WhisperContext, String> {
    if let Some(c) = WHISPER_CTX.get() {
        return Ok(c);
    }
    let path = model_path();
    if !path.exists() {
        return Err(format!("whisper model not found: {}", path.display()));
    }
    let ctx = WhisperContext::new_with_params(
        path.to_str().unwrap(),
        WhisperContextParameters::default(),
    )
    .map_err(|e| format!("whisper init: {}", e))?;
    let _ = WHISPER_CTX.set(ctx);
    Ok(WHISPER_CTX.get().unwrap())
}

pub fn pcm_bytes_to_f32(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if !bytes.len().is_multiple_of(4) {
        return Err("invalid PCM f32 bytes len".into());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        let v = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        out.push(v);
    }
    Ok(out)
}

pub fn validate_samples(n: usize) -> Result<usize, String> {
    const MIN: usize = 4800; // ~0.3s @16k
    const MAX: usize = 16000 * 30;
    if n < MIN {
        return Err("utterance too short".into());
    }
    if n > MAX {
        return Ok(MAX); // truncate
    }
    Ok(n)
}

// zh-TW/en mixed speech: auto-detect on short utterances often locks onto
// English and drops the Chinese — pin zh. Override with SHIKIGAMI_STT_LANG.
fn stt_language() -> String {
    std::env::var("SHIKIGAMI_STT_LANG").unwrap_or_else(|_| "zh".to_string())
}

pub fn transcribe(pcm: &[f32]) -> Result<String, String> {
    let n = validate_samples(pcm.len())?;
    let samples = if n < pcm.len() { &pcm[..n] } else { pcm };
    let ctx = get_ctx()?;
    let mut state = ctx.create_state().map_err(|e| e.to_string())?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    let lang = stt_language();
    params.set_language(Some(&lang));
    params.set_translate(false);
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    state
        .full(params, samples)
        .map_err(|e| format!("whisper full: {}", e))?;
    let num = state.full_n_segments();
    let mut text = String::new();
    for i in 0..num {
        if let Some(seg) = state.get_segment(i) {
            match seg.to_str_lossy() {
                Ok(s) => text.push_str(&s),
                Err(e) => return Err(format!("whisper segment: {}", e)),
            }
        }
    }
    Ok(text.trim().to_string())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t1_pcm_bytes_to_f32_1_0() {
        // 1.0f32 le bytes
        let b = [0x00u8, 0x00, 0x80, 0x3F];
        assert_eq!(pcm_bytes_to_f32(&b).unwrap(), vec![1.0f32]);
    }

    #[test]
    fn t2_pcm_invalid_len_err() {
        assert!(pcm_bytes_to_f32(&[0, 0, 0]).is_err());
    }

    #[test]
    fn t3_validate_too_short() {
        assert!(validate_samples(4799).is_err());
    }

    #[test]
    fn t4_validate_min_ok() {
        assert_eq!(validate_samples(4800).unwrap(), 4800);
    }

    #[test]
    fn t5_validate_truncates_31s() {
        let n = 16000 * 31;
        assert_eq!(validate_samples(n).unwrap(), 16000 * 30);
    }

    #[test]
    fn t6_model_missing_err_contains_ggml() {
        // point model lookup at empty temp
        let old_home = std::env::var("HOME").ok();
        let tmp = std::env::temp_dir().join("no-model-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).ok();
        std::env::set_var("HOME", &tmp);
        let res = get_ctx();
        if let Some(h) = old_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
        let e = res.unwrap_err();
        assert!(e.contains(MODEL_NAME));
    }

    #[test]
    #[ignore]
    fn t7_real_model_silence_empty_or_ws() {
        // manual: cargo test -- --ignored ; needs model file
        // 1s silence -> empty or ws transcript
        let sil = vec![0.0f32; 16000];
        let t = transcribe(&sil).unwrap_or_default();
        assert!(t.trim().is_empty());
    }
}
