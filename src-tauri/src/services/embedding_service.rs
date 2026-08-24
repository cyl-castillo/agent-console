//! Local text embeddings for semantic memory/skill recall.
//!
//! Model: intfloat/multilingual-e5-small, run with candle — pure-Rust
//! inference on CPU. The first choice here was fastembed/onnxruntime, but the
//! prebuilt ONNX Runtime binaries require glibc ≥ 2.38 and our builds (and
//! users) target Ubuntu 22.04 (glibc 2.35); candle removes native libraries
//! from the equation entirely, on every platform.
//!
//! Model files (~470MB, fp32 safetensors) download once from the HF CDN into
//! the app cache — the Whisper pattern. The model is loaded per operation from
//! mmapped safetensors (fast, and it keeps resident memory low when idle)
//! instead of living in RAM for the app's lifetime.
//!
//! The `Embedder` trait exists so index logic is testable without the model:
//! tests use a deterministic fake; production uses `CandleEmbedder`.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use tokenizers::Tokenizer;

use crate::error::{AppError, AppResult};

/// E5-family models are trained with these role prefixes; skipping them
/// measurably degrades retrieval quality.
const PASSAGE_PREFIX: &str = "passage: ";
const QUERY_PREFIX: &str = "query: ";

const MODEL_REPO: &str = "intfloat/multilingual-e5-small";
const MODEL_FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];
const MAX_TOKENS: usize = 512;

/// Bump when the model (or prefixing/pooling scheme) changes — stored in the
/// index so stale vectors are re-embedded instead of silently compared across
/// schemes.
pub const EMBEDDING_SCHEME: &str = "multilingual-e5-small/candle-v1";

pub trait Embedder: Send {
    /// Embed documents (passages). One vector per input, all the same length.
    fn embed_passages(&mut self, texts: &[String]) -> AppResult<Vec<Vec<f32>>>;
    /// Embed a search query.
    fn embed_query(&mut self, text: &str) -> AppResult<Vec<f32>>;
}

fn model_dir() -> AppResult<PathBuf> {
    let dir = dirs::cache_dir()
        .ok_or_else(|| AppError::Other("no cache dir".into()))?
        .join("agent-console")
        .join("embeddings")
        .join("multilingual-e5-small");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Download any missing model file (temp + rename so a cut connection never
/// leaves a truncated file behind to "succeed" later).
fn ensure_model_files() -> AppResult<PathBuf> {
    let dir = model_dir()?;
    for file in MODEL_FILES {
        let dest = dir.join(file);
        if dest.exists() {
            continue;
        }
        let url = format!("https://huggingface.co/{MODEL_REPO}/resolve/main/{file}");
        let resp = reqwest::blocking::Client::builder()
            .user_agent("agent-console")
            .timeout(std::time::Duration::from_secs(1800))
            .build()
            .map_err(|e| AppError::Other(format!("http client: {e}")))?
            .get(&url)
            .send()
            .map_err(|e| AppError::Other(format!("model download failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "model download for {file} returned {}",
                resp.status()
            )));
        }
        let bytes = resp
            .bytes()
            .map_err(|e| AppError::Other(format!("model download failed: {e}")))?;
        let tmp = dir.join(format!("{file}.part"));
        fs::write(&tmp, &bytes)?;
        fs::rename(&tmp, &dest)?;
    }
    Ok(dir)
}

/// Whether the embedding model is already on disk, without ever downloading.
/// The memory-injection path runs on every prompt and must answer fast from
/// local state only — the ~470MB download belongs to the explicit
/// reindex/search flows, never to a prompt submission.
pub fn model_ready() -> bool {
    let Ok(dir) = model_dir() else { return false };
    MODEL_FILES.iter().all(|f| dir.join(f).exists())
}

/// Pure-Rust embedder. Loads tokenizer + mmapped weights per instance; create
/// one per operation and let it drop — cheap to construct, low idle memory.
pub struct CandleEmbedder {
    inner: Option<(Tokenizer, BertModel)>,
}

/// Process-wide embedder for latency-sensitive paths. The inject endpoint
/// lives inside the hook's ~1.5s budget, and a fresh BERT load alone costs
/// seconds — so the hot path loads the model once and keeps it resident.
/// Batch paths (reindexing) keep their own short-lived instances.
static SHARED: Mutex<Option<CandleEmbedder>> = Mutex::new(None);

pub fn with_shared_embedder<T>(f: impl FnOnce(&mut CandleEmbedder) -> T) -> T {
    let mut guard = SHARED.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(CandleEmbedder::new))
}

impl CandleEmbedder {
    pub fn new() -> Self {
        Self { inner: None }
    }

    /// Force the lazy model load now — called off the critical path at startup
    /// so the first real query doesn't pay the multi-second load.
    pub fn warm(&mut self) -> AppResult<()> {
        self.ensure_loaded().map(|_| ())
    }

    fn ensure_loaded(&mut self) -> AppResult<&(Tokenizer, BertModel)> {
        if self.inner.is_none() {
            let dir = ensure_model_files()?;
            let config: Config =
                serde_json::from_str(&fs::read_to_string(dir.join("config.json"))?)
                    .map_err(|e| AppError::Other(format!("model config: {e}")))?;
            let tokenizer = Tokenizer::from_file(dir.join("tokenizer.json"))
                .map_err(|e| AppError::Other(format!("tokenizer: {e}")))?;
            let vb = unsafe {
                VarBuilder::from_mmaped_safetensors(
                    &[dir.join("model.safetensors")],
                    DType::F32,
                    &Device::Cpu,
                )
            }
            .map_err(|e| AppError::Other(format!("model weights: {e}")))?;
            let model = BertModel::load(vb, &config)
                .map_err(|e| AppError::Other(format!("model load: {e}")))?;
            self.inner = Some((tokenizer, model));
        }
        Ok(self.inner.as_ref().expect("loaded above"))
    }

    /// Tokenize, forward, mask-aware mean-pool. Batched in small chunks so a
    /// big reindex doesn't build one huge padded tensor.
    fn embed_batch(&mut self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(8) {
            let (tokenizer, model) = self.ensure_loaded()?;
            let mut encodings = Vec::with_capacity(chunk.len());
            for t in chunk {
                let mut enc = tokenizer
                    .encode(t.as_str(), true)
                    .map_err(|e| AppError::Other(format!("tokenize: {e}")))?;
                enc.truncate(MAX_TOKENS, 0, tokenizers::TruncationDirection::Right);
                encodings.push(enc);
            }
            let max_len = encodings.iter().map(|e| e.len()).max().unwrap_or(1).max(1);
            let mut ids: Vec<u32> = Vec::with_capacity(chunk.len() * max_len);
            let mut mask: Vec<u32> = Vec::with_capacity(chunk.len() * max_len);
            for enc in &encodings {
                ids.extend(enc.get_ids());
                mask.extend(enc.get_attention_mask());
                let pad = max_len - enc.len();
                ids.extend(std::iter::repeat_n(0, pad));
                mask.extend(std::iter::repeat_n(0, pad));
            }
            let device = Device::Cpu;
            let shape = (chunk.len(), max_len);
            let input_ids = Tensor::from_vec(ids, shape, &device)
                .map_err(|e| AppError::Other(format!("tensor: {e}")))?;
            let attention = Tensor::from_vec(mask, shape, &device)
                .map_err(|e| AppError::Other(format!("tensor: {e}")))?;
            let token_type = input_ids
                .zeros_like()
                .map_err(|e| AppError::Other(format!("tensor: {e}")))?;
            let hidden = model
                .forward(&input_ids, &token_type, Some(&attention))
                .map_err(|e| AppError::Other(format!("forward: {e}")))?;
            // Mask-aware mean pooling: sum(hidden * mask) / sum(mask).
            let maskf = attention
                .to_dtype(DType::F32)
                .and_then(|m| m.unsqueeze(2))
                .map_err(|e| AppError::Other(format!("pool: {e}")))?;
            let pooled = hidden
                .broadcast_mul(&maskf)
                .and_then(|h| h.sum(1))
                .map_err(|e| AppError::Other(format!("pool: {e}")))?;
            let counts = maskf
                .sum(1)
                .and_then(|c| c.clamp(1e-9, f64::INFINITY))
                .map_err(|e| AppError::Other(format!("pool: {e}")))?;
            let mean = pooled
                .broadcast_div(&counts)
                .map_err(|e| AppError::Other(format!("pool: {e}")))?;
            let vecs: Vec<Vec<f32>> = mean
                .to_vec2()
                .map_err(|e| AppError::Other(format!("pool: {e}")))?;
            out.extend(vecs);
        }
        Ok(out)
    }
}

impl Default for CandleEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

impl Embedder for CandleEmbedder {
    fn embed_passages(&mut self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let prefixed: Vec<String> = texts
            .iter()
            .map(|t| format!("{PASSAGE_PREFIX}{t}"))
            .collect();
        self.embed_batch(&prefixed)
    }

    fn embed_query(&mut self, text: &str) -> AppResult<Vec<f32>> {
        let mut out = self.embed_batch(&[format!("{QUERY_PREFIX}{text}")])?;
        out.pop()
            .ok_or_else(|| AppError::Other("embedding model returned nothing".into()))
    }
}

/// Cosine similarity; inputs need not be pre-normalized.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_basics() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
        // Degenerate inputs are 0, never NaN.
        assert_eq!(cosine(&[], &[]), 0.0);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    /// Real end-to-end smoke test: downloads the model (~470MB) on first run.
    /// Ignored by default — run explicitly with:
    ///   cargo test real_model_smoke -- --ignored
    #[test]
    #[ignore]
    fn real_model_smoke() {
        let mut e = CandleEmbedder::new();
        let vecs = e
            .embed_passages(&[
                "el guardado de sesiones falla en Windows".to_string(),
                "session persistence breaks on Windows".to_string(),
                "receta de pan casero con masa madre".to_string(),
            ])
            .expect("embed passages");
        assert_eq!(vecs.len(), 3);
        assert_eq!(vecs[0].len(), 384, "e5-small hidden size");
        let q = e
            .embed_query("bug de persistencia de sesiones en windows")
            .expect("embed query");
        let s_es = cosine(&q, &vecs[0]);
        let s_en = cosine(&q, &vecs[1]);
        let s_off = cosine(&q, &vecs[2]);
        // Cross-language relevance must beat the off-topic doc, decisively.
        assert!(s_es > s_off + 0.05, "es {s_es} vs off {s_off}");
        assert!(s_en > s_off + 0.05, "en {s_en} vs off {s_off}");
    }
}
