//! Embedding strategies for the vector store.
//!
//! An [`Embedder`] turns a piece of text into a fixed-length float vector. The
//! store compares vectors with cosine similarity ([`CosineSim`]), so any
//! embedding scheme that places semantically-similar text close together in
//! vector space works.

/// Turn text into a fixed-length embedding vector.
pub trait Embedder {
    /// Embed `text` into the vector space. Implementations must be deterministic
    /// (same text -> same vector) so indexes are reproducible.
    fn embed(&self, text: &str) -> Vec<f32>;
    /// The dimensionality of vectors produced by [`Embedder::embed`].
    fn dim(&self) -> usize;
}

/// Cosine similarity between two equal-length vectors in `[-1, 1]`.
///
/// Returns `0.0` for degenerate (all-zero) inputs rather than dividing by zero.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let norm = na.sqrt() * nb.sqrt();
    if norm < 1e-9 {
        return 0.0;
    }
    (dot / norm).clamp(-1.0, 1.0)
}

/// A deterministic, model-free embedder based on feature hashing
/// (the "hashing trick").
///
/// Each token is hashed into one of `dim` buckets and its count accumulated.
/// The result is L2-normalized so retrieval depends on token *proportions*,
/// not raw length. Two documents sharing vocabulary land close together, which
/// is enough for in-project code/doc retrieval without a neural model.
pub struct FeatureHashEmbedder {
    dim: usize,
}

impl FeatureHashEmbedder {
    /// Create an embedder with `dim` hash buckets (more buckets -> sparser,
    /// lower collision; 256 is a reasonable default for small projects).
    pub fn new(dim: usize) -> Self {
        FeatureHashEmbedder { dim: dim.max(1) }
    }
}

impl Embedder for FeatureHashEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        let mut vec = vec![0.0f32; self.dim];
        for token in tokenize(text) {
            let h = fnv1a(token.as_bytes());
            let bucket = (h as usize) % self.dim;
            vec[bucket] += 1.0;
        }
        // L2 normalize.
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 1e-9 {
            for v in vec.iter_mut() {
                *v /= norm;
            }
        }
        vec
    }

    fn dim(&self) -> usize {
        self.dim
    }
}

/// Split text into lower-cased alphanumeric tokens.
pub fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

/// FNV-1a 64-bit hash — fast, dependency-free, deterministic.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_of_identical_vectors_is_one() {
        let a = [1.0f32, 2.0, 3.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_of_orthogonal_vectors_is_zero() {
        let a = [1.0f32, 0.0];
        let b = [0.0f32, 1.0];
        assert!((cosine_similarity(&a, &b)).abs() < 1e-6);
    }

    #[test]
    fn cosine_handles_mismatched_lengths() {
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn feature_hash_is_deterministic_and_normalized() {
        let e = FeatureHashEmbedder::new(64);
        let v1 = e.embed("the quick brown fox");
        let v2 = e.embed("the quick brown fox");
        assert_eq!(v1, v2);
        let norm: f32 = v1.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
        assert_eq!(v1.len(), 64);
    }

    #[test]
    fn similar_text_embeds_closer_than_dissimilar() {
        let e = FeatureHashEmbedder::new(128);
        let a = e.embed("physics body collider rapier integration step");
        let b = e.embed("collider body rapier physics simulation step");
        let c = e.embed("romantic poetry about the sea at midnight");
        let sim_ab = cosine_similarity(&a, &b);
        let sim_ac = cosine_similarity(&a, &c);
        assert!(sim_ab > sim_ac, "similar docs should score higher");
    }

    #[test]
    fn tokenize_strips_punctuation() {
        let toks = tokenize("Hello, World! foo_bar-baz.");
        assert!(toks.contains(&"hello".to_string()));
        assert!(toks.contains(&"world".to_string()));
        assert!(toks.contains(&"foo".to_string()));
        assert!(toks.contains(&"bar".to_string()));
        assert!(toks.contains(&"baz".to_string()));
    }
}
