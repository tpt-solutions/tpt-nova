//! Local vector-DB and retrieval-augmented-generation (RAG) context for Nova.
//!
//! `nova-rag` gives AI agents and in-editor tooling a way to *ask the project*:
//! index the source tree, assets, and docs into a small in-memory vector store,
//! then retrieve the most relevant chunks for a natural-language query. It is
//! intentionally dependency-free of any external model — embeddings come from a
//! pluggable [`Embedder`]; the crate ships a deterministic
//! [`FeatureHashEmbedder`] (feature-hashing bag-of-words) so it works offline
//! and under CI, and a production deployment can drop in a real sentence/model
//! embedder behind the same trait.

pub mod embed;
pub mod index;

pub use embed::{cosine_similarity, Embedder, FeatureHashEmbedder};
pub use index::{Document, Index, RagAgent, ScoredHit, SearchError};

/// Convenience: build an [`Index`] from a directory of text-like files.
///
/// `extensions` filters by lower-cased file extension (e.g. `["rs", "md",
/// "toml"]`); an empty list indexes every file. Hidden directories (those
/// starting with `.`) are skipped.
pub fn index_directory<P: AsRef<std::path::Path>>(
    root: P,
    extensions: &[&str],
) -> Result<Index, SearchError> {
    index::index_directory(root, extensions)
}
