//! Local text embeddings for semantic memory/skill recall.
//!
//! Model: multilingual-e5-small via fastembed (ONNX, CPU). ~100MB, downloaded
//! once into the app cache on first use — same pattern as the Whisper model
//! for voice input. Nothing ever leaves the machine.
//!
//! The `Embedder` trait exists so index logic is testable without the model:
//! tests use a deterministic fake; production uses the lazy fastembed handle.

use parking_lot::Mutex;
use std::path::PathBuf;

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

use crate::error::{AppError, AppResult};

/// E5-family models are trained with these role prefixes; skipping them
/// measurably degrades retrieval quality.
const PASSAGE_PREFIX: &str = "passage: ";
const QUERY_PREFIX: &str = "query: ";

/// Bump when the model (or prefixing scheme) changes — stored in the index so
/// stale vectors are re-embedded instead of silently compared across models.
pub const EMBEDDING_SCHEME: &str = "multilingual-e5-small/v1";

pub trait Embedder: Send {
    /// Embed documents (passages). One vector per input, all the same length.
    fn embed_passages(&mut self, texts: &[String]) -> AppResult<Vec<Vec<f32>>>;
    /// Embed a search query.
    fn embed_query(&mut self, text: &str) -> AppResult<Vec<f32>>;
}

/// Lazily-initialized global model. Init downloads the model on first use
/// (~100MB, minutes on slow links) — callers run inside async commands, never
/// on the UI thread.
static MODEL: Mutex<Option<TextEmbedding>> = Mutex::new(None);

fn model_cache_dir() -> AppResult<PathBuf> {
    let dir = dirs::cache_dir()
        .ok_or_else(|| AppError::Other("no cache dir".into()))?
        .join("agent-console")
        .join("embeddings");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub struct FastembedEmbedder;

impl FastembedEmbedder {
    fn with_model<T>(f: impl FnOnce(&mut TextEmbedding) -> AppResult<T>) -> AppResult<T> {
        let mut guard = MODEL.lock();
        if guard.is_none() {
            let options = TextInitOptions::new(EmbeddingModel::MultilingualE5Small)
                .with_cache_dir(model_cache_dir()?)
                .with_show_download_progress(false)
                // Half the cores: embedding runs alongside live agent sessions;
                // saturating the CPU for a background index isn't worth it.
                .with_intra_threads(
                    (std::thread::available_parallelism().map_or(2, |n| n.get()) / 2).max(1),
                );
            let model = TextEmbedding::try_new(options)
                .map_err(|e| AppError::Other(format!("embedding model init: {e}")))?;
            *guard = Some(model);
        }
        f(guard.as_mut().expect("model initialized above"))
    }
}

impl Embedder for FastembedEmbedder {
    fn embed_passages(&mut self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let prefixed: Vec<String> = texts
            .iter()
            .map(|t| format!("{PASSAGE_PREFIX}{t}"))
            .collect();
        Self::with_model(|m| {
            m.embed(&prefixed, None)
                .map_err(|e| AppError::Other(format!("embed: {e}")))
        })
    }

    fn embed_query(&mut self, text: &str) -> AppResult<Vec<f32>> {
        let prefixed = vec![format!("{QUERY_PREFIX}{text}")];
        let mut out = Self::with_model(|m| {
            m.embed(&prefixed, None)
                .map_err(|e| AppError::Other(format!("embed: {e}")))
        })?;
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
}
