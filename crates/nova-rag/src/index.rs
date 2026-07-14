//! The vector index and RAG query layer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::embed::{cosine_similarity, Embedder, FeatureHashEmbedder};

/// Errors raised by indexing and search.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("index is empty")]
    EmptyIndex,
    #[error("query produced no usable embedding")]
    EmptyQuery,
}

/// A single indexed unit of text. For source trees this is typically one file
/// (or a chunk of a large file); for docs, one section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Document {
    pub id: String,
    pub text: String,
    /// Free-form metadata (file path, title, kind, line range, ...).
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl Document {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Document {
            id: id.into(),
            text: text.into(),
            metadata: HashMap::new(),
        }
    }

    /// Number of alphanumeric tokens in the document text.
    pub fn token_count(&self) -> usize {
        crate::embed::tokenize(&self.text).len()
    }
}

/// A document plus its precomputed embedding, stored together in the index so
/// search never re-embeds.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    doc: Document,
    vector: Vec<f32>,
}

/// A search hit: a document and its similarity score to the query.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredHit {
    pub document: Document,
    /// Cosine similarity in `[-1, 1]`; higher is more relevant.
    pub score: f32,
}

/// An in-memory cosine-similarity vector index.
pub struct Index {
    entries: Vec<Entry>,
    embedder: Box<dyn Embedder>,
}

impl Index {
    /// Create an empty index using the given embedder.
    pub fn new(embedder: Box<dyn Embedder>) -> Self {
        Index {
            entries: Vec::new(),
            embedder,
        }
    }

    /// Create an index backed by the default offline [`FeatureHashEmbedder`]
    /// (256 dimensions).
    pub fn default_new() -> Self {
        Self::new(Box::new(FeatureHashEmbedder::new(256)))
    }

    /// Number of documents in the index.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Add one document, embedding it immediately.
    pub fn add(&mut self, doc: Document) {
        let vector = self.embedder.embed(&doc.text);
        self.entries.push(Entry { doc, vector });
    }

    /// Add many documents at once.
    pub fn add_documents(&mut self, docs: impl IntoIterator<Item = Document>) {
        for d in docs {
            self.add(d);
        }
    }

    /// Return the `k` most similar documents to `query`, highest score first.
    ///
    /// Ties are broken by document id for deterministic output.
    pub fn search(&self, query: &str, k: usize) -> Result<Vec<ScoredHit>, SearchError> {
        if self.entries.is_empty() {
            return Err(SearchError::EmptyIndex);
        }
        let qv = self.embedder.embed(query);
        if qv.iter().all(|v| *v == 0.0) {
            return Err(SearchError::EmptyQuery);
        }
        let mut scored: Vec<ScoredHit> = self
            .entries
            .iter()
            .map(|e| ScoredHit {
                score: cosine_similarity(&qv, &e.vector),
                document: e.doc.clone(),
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.document.id.cmp(&b.document.id))
        });
        let k = k.min(scored.len());
        Ok(scored.into_iter().take(k).collect())
    }

    /// Serialize the index (documents + embeddings) to compact JSON.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), SearchError> {
        let json = serde_json::to_string(&self.entries)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load a previously saved index. The embedder type is reconstructed from
    /// the stored vectors' dimensionality using the default feature-hash
    /// embedder (the stored vectors are reused verbatim — no re-embedding).
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, SearchError> {
        let text = std::fs::read_to_string(path)?;
        let entries: Vec<Entry> = serde_json::from_str(&text)?;
        let dim = entries.first().map(|e| e.vector.len()).unwrap_or(256);
        Ok(Index {
            entries,
            embedder: Box::new(FeatureHashEmbedder::new(dim)),
        })
    }
}

/// A retrieval-augmented-generation helper: wraps an [`Index`] and assembles a
/// prompt-ready context block from the top search hits, so an LLM/agent can be
/// grounded in the actual project without the engine reimplementing the glue.
pub struct RagAgent {
    index: Index,
    top_k: usize,
}

impl RagAgent {
    pub fn new(index: Index, top_k: usize) -> Self {
        RagAgent {
            index,
            top_k: top_k.max(1),
        }
    }

    /// Retrieve the top-`k` documents for `query`.
    pub fn retrieve(&self, query: &str) -> Result<Vec<ScoredHit>, SearchError> {
        self.index.search(query, self.top_k)
    }

    /// Build a context string for an LLM prompt: each retrieved chunk prefixed
    /// with its source id and similarity score.
    pub fn build_context(&self, query: &str) -> Result<String, SearchError> {
        let hits = self.retrieve(query)?;
        if hits.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::new();
        out.push_str("Relevant project context:\n");
        for (i, hit) in hits.iter().enumerate() {
            let source = hit
                .document
                .metadata
                .get("path")
                .cloned()
                .unwrap_or_else(|| hit.document.id.clone());
            out.push_str(&format!(
                "\n[{i}] (score={:.3}, source={}) {}\n",
                hit.score, source, hit.document.text
            ));
        }
        Ok(out)
    }
}

/// Recursively index a directory, adding one [`Document`] per file whose
/// extension is in `extensions` (empty = all files). Hidden directories (`.*`)
/// are skipped.
pub fn index_directory(root: impl AsRef<Path>, extensions: &[&str]) -> Result<Index, SearchError> {
    let root = root.as_ref();
    let mut index = Index::default_new();
    let mut paths: Vec<PathBuf> = Vec::new();
    collect_files(root, &mut paths);

    let ext_set: Vec<String> = extensions.iter().map(|e| e.to_ascii_lowercase()).collect();
    for path in paths {
        let is_allowed = ext_set.is_empty()
            || path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| ext_set.contains(&e.to_ascii_lowercase()))
                .unwrap_or(false);
        if !is_allowed {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let mut doc = Document::new(rel.clone(), text);
        doc.metadata.insert("path".to_string(), rel);
        index.add(doc);
    }
    Ok(index)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Skip hidden / VCS / build dirs to keep the index focused.
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }
            }
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_index() -> Index {
        let mut idx = Index::default_new();
        idx.add_documents([
            Document::new("a", "physics body collider rapier integration step"),
            Document::new("b", "render wgpu pipeline shader vertex fragment"),
            Document::new("c", "scene save load serialization ron json version"),
        ]);
        idx
    }

    #[test]
    fn search_ranks_relevant_first() {
        let idx = sample_index();
        let hits = idx.search("physics collider rapier", 2).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].document.id, "a");
    }

    #[test]
    fn search_empty_index_errors() {
        let idx = Index::default_new();
        assert!(matches!(idx.search("x", 1), Err(SearchError::EmptyIndex)));
    }

    #[test]
    fn save_and_load_roundtrips() {
        let dir = std::env::temp_dir();
        let path = dir.join("nova_rag_test.json");
        let idx = sample_index();
        idx.save(&path).unwrap();
        let loaded = Index::load(&path).unwrap();
        assert_eq!(loaded.len(), idx.len());
        let hits = loaded.search("wgpu shader pipeline", 1).unwrap();
        assert_eq!(hits[0].document.id, "b");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rag_agent_builds_context_block() {
        let idx = sample_index();
        let agent = RagAgent::new(idx, 2);
        let ctx = agent.build_context("scene serialization").unwrap();
        assert!(ctx.contains("scene save load"));
        assert!(ctx.contains("score="));
    }

    #[test]
    fn directory_indexing_counts_files() {
        let dir = std::env::temp_dir().join("nova_rag_dir_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("one.rs"), "physics collider body").unwrap();
        std::fs::write(dir.join("two.md"), "render shader wgpu").unwrap();
        std::fs::write(dir.join("skip.txt"), "should be ignored").unwrap();
        let idx = index_directory(&dir, &["rs", "md"]).unwrap();
        assert_eq!(idx.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_indexing_skips_hidden_dirs() {
        let dir = std::env::temp_dir().join("nova_rag_hidden_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git").join("x.rs"), "hidden file").unwrap();
        std::fs::write(dir.join("visible.rs"), "visible file physics").unwrap();
        let idx = index_directory(&dir, &["rs"]).unwrap();
        assert_eq!(idx.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
