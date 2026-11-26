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

/// Errors from [`RealEmbedder`].
#[cfg(feature = "real-embeddings")]
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("invalid endpoint `{0}`: expected http://host[:port][/path] (https is not supported)")]
    InvalidEndpoint(String),
    #[error("embedding request failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("embedding server returned an error: {0}")]
    Server(String),
    #[error("malformed embedding response: {0}")]
    BadResponse(String),
}

/// A neural embedder that calls a **local** embeddings server over plain HTTP.
///
/// Unlike [`FeatureHashEmbedder`] (a model-free bag-of-words baseline), the
/// vectors come from a sentence-embedding model such as All-MiniLM-L6-v2, which
/// captures *semantic* similarity. The model runs in a separate process you
/// control (for example Ollama or `llama-server --embedding`), so this crate
/// ships no model weights, no inference runtime and no TLS stack.
///
/// The client speaks the OpenAI-compatible `POST /v1/embeddings` protocol, which
/// both servers implement. Only `http://` endpoints are accepted: TLS would drag
/// in a crypto provider this project does not ship, and a local server does not
/// need it. The vector dimension is learned from one probe request in
/// [`RealEmbedder::new`].
///
/// ```no_run
/// # use tpt_nova_rag::{Index, RealEmbedder};
/// let e = RealEmbedder::new("http://127.0.0.1:11434/v1/embeddings", "all-minilm")?;
/// let index = Index::new(Box::new(e));
/// # Ok::<(), tpt_nova_rag::embed::EmbedError>(())
/// ```
#[cfg(feature = "real-embeddings")]
pub struct RealEmbedder {
    /// `host:port` for the TCP connection and the `Host` header.
    authority: String,
    path: String,
    model: String,
    dim: usize,
    timeout: std::time::Duration,
}

#[cfg(feature = "real-embeddings")]
impl RealEmbedder {
    /// Connect to `endpoint` (e.g. `http://127.0.0.1:11434/v1/embeddings`) and
    /// probe it once to learn the vector dimension.
    pub fn new(endpoint: &str, model: &str) -> Result<Self, EmbedError> {
        let (authority, path) = parse_http_endpoint(endpoint)?;
        let mut e = Self {
            authority,
            path,
            model: model.to_string(),
            dim: 0,
            timeout: std::time::Duration::from_secs(30),
        };
        let probe = e.request("probe")?;
        if probe.is_empty() {
            return Err(EmbedError::BadResponse(
                "server returned an empty vector".into(),
            ));
        }
        e.dim = probe.len();
        Ok(e)
    }

    /// Embed `text`, returning an error instead of an empty vector.
    pub fn try_embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        self.request(text)
    }

    fn request(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let body = serde_json::json!({ "model": self.model, "input": text }).to_string();
        let mut stream = std::net::TcpStream::connect(&self.authority)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        use std::io::{Read, Write};
        write!(
            stream,
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.path,
            self.authority,
            body.len(),
            body
        )?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw)?;
        let (status, chunked, payload) = split_http_response(&raw)?;
        let payload = if chunked { dechunk(&payload)? } else { payload };
        if status != 200 {
            let snippet = String::from_utf8_lossy(&payload);
            return Err(EmbedError::Server(format!(
                "HTTP {status}: {}",
                snippet.trim()
            )));
        }
        parse_embedding(&payload)
    }
}

#[cfg(feature = "real-embeddings")]
impl Embedder for RealEmbedder {
    /// Returns an empty vector if the request fails; use
    /// [`RealEmbedder::try_embed`] to see the error.
    fn embed(&self, text: &str) -> Vec<f32> {
        self.request(text).unwrap_or_default()
    }

    fn dim(&self) -> usize {
        self.dim
    }
}

/// Split `http://host[:port][/path]` into `(host:port, path)`.
#[cfg(feature = "real-embeddings")]
fn parse_http_endpoint(endpoint: &str) -> Result<(String, String), EmbedError> {
    let rest = endpoint
        .strip_prefix("http://")
        .ok_or_else(|| EmbedError::InvalidEndpoint(endpoint.to_string()))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/v1/embeddings"),
    };
    if authority.is_empty() {
        return Err(EmbedError::InvalidEndpoint(endpoint.to_string()));
    }
    let authority = if authority.contains(':') {
        authority.to_string()
    } else {
        format!("{authority}:80")
    };
    Ok((authority, path.to_string()))
}

/// Split a raw HTTP/1.1 response into `(status, is_chunked, body)`.
#[cfg(feature = "real-embeddings")]
fn split_http_response(raw: &[u8]) -> Result<(u16, bool, Vec<u8>), EmbedError> {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| EmbedError::BadResponse("no header terminator".into()))?;
    let head = String::from_utf8_lossy(&raw[..sep]).to_ascii_lowercase();
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| EmbedError::BadResponse("no status line".into()))?;
    let chunked = head
        .lines()
        .any(|l| l.starts_with("transfer-encoding:") && l.contains("chunked"));
    Ok((status, chunked, raw[sep + 4..].to_vec()))
}

/// Decode a `Transfer-Encoding: chunked` body.
#[cfg(feature = "real-embeddings")]
fn dechunk(mut body: &[u8]) -> Result<Vec<u8>, EmbedError> {
    let bad = || EmbedError::BadResponse("bad chunked encoding".into());
    let mut out = Vec::new();
    loop {
        let eol = body.windows(2).position(|w| w == b"\r\n").ok_or_else(bad)?;
        let size_str = std::str::from_utf8(&body[..eol]).map_err(|_| bad())?;
        let size_hex = size_str.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16).map_err(|_| bad())?;
        body = &body[eol + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size + 2 {
            return Err(bad());
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

/// Read the vector from an OpenAI-style `{"data":[{"embedding":[..]}]}` reply,
/// or an Ollama-style `{"embeddings":[[..]]}` reply.
#[cfg(feature = "real-embeddings")]
fn parse_embedding(payload: &[u8]) -> Result<Vec<f32>, EmbedError> {
    let v: serde_json::Value = serde_json::from_slice(payload)
        .map_err(|e| EmbedError::BadResponse(format!("invalid JSON: {e}")))?;
    let vector = v
        .pointer("/data/0/embedding")
        .or_else(|| v.pointer("/embeddings/0"))
        .and_then(|a| a.as_array())
        .ok_or_else(|| EmbedError::BadResponse("no embedding array in reply".into()))?;
    vector
        .iter()
        .map(|x| {
            x.as_f64()
                .map(|f| f as f32)
                .ok_or_else(|| EmbedError::BadResponse("non-numeric embedding value".into()))
        })
        .collect()
}

#[cfg(all(test, feature = "real-embeddings"))]
mod real_embedder_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serve `connections` HTTP requests on a loopback port, answering each with
    /// `status` and `body` (optionally chunk-encoded). Returns the endpoint URL.
    fn mock_server(connections: usize, status: u16, body: &'static str, chunked: bool) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for _ in 0..connections {
                let (mut s, _) = listener.accept().unwrap();
                // Read until the request body (per Content-Length) has arrived.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                loop {
                    let n = s.read(&mut tmp).unwrap();
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(sep) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..sep]).to_ascii_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        if buf.len() >= sep + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                let payload = if chunked {
                    format!("{:x}\r\n{}\r\n0\r\n\r\n", body.len(), body)
                } else {
                    body.to_string()
                };
                let te = if chunked {
                    "Transfer-Encoding: chunked\r\n"
                } else {
                    ""
                };
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{te}Connection: close\r\n\r\n{payload}"
                );
                s.write_all(resp.as_bytes()).unwrap();
            }
        });
        format!("http://{addr}/v1/embeddings")
    }

    const VEC_REPLY: &str = r#"{"object":"list","data":[{"embedding":[0.5,0.5,0.5,0.5]}]}"#;

    #[test]
    fn new_probes_dimension_and_embeds_via_openai_protocol() {
        // One connection for the probe in `new`, one for `embed`.
        let url = mock_server(2, 200, VEC_REPLY, false);
        let e = RealEmbedder::new(&url, "test-model").unwrap();
        assert_eq!(e.dim(), 4);
        assert_eq!(e.embed("hello"), vec![0.5; 4]);
    }

    #[test]
    fn accepts_chunked_transfer_encoding() {
        let url = mock_server(1, 200, VEC_REPLY, true);
        let e = RealEmbedder::new(&url, "m").unwrap();
        assert_eq!(e.dim(), 4);
    }

    #[test]
    fn accepts_ollama_style_reply() {
        let url = mock_server(1, 200, r#"{"embeddings":[[1.0,2.0,3.0]]}"#, false);
        let e = RealEmbedder::new(&url, "m").unwrap();
        assert_eq!(e.dim(), 3);
    }

    #[test]
    fn server_error_status_is_reported() {
        let url = mock_server(1, 500, "model not loaded", false);
        match RealEmbedder::new(&url, "m") {
            Err(EmbedError::Server(msg)) => assert!(msg.contains("model not loaded")),
            other => panic!("expected Server error, got {:?}", other.err()),
        }
    }

    #[test]
    fn https_endpoints_are_rejected_without_connecting() {
        assert!(matches!(
            RealEmbedder::new("https://example.invalid/v1/embeddings", "m"),
            Err(EmbedError::InvalidEndpoint(_))
        ));
    }

    #[test]
    fn parse_endpoint_defaults_port_and_path() {
        let (auth, path) = parse_http_endpoint("http://localhost").unwrap();
        assert_eq!(auth, "localhost:80");
        assert_eq!(path, "/v1/embeddings");
        let (auth, path) = parse_http_endpoint("http://127.0.0.1:11434/api/embed").unwrap();
        assert_eq!(auth, "127.0.0.1:11434");
        assert_eq!(path, "/api/embed");
    }

    #[test]
    #[ignore = "needs a running local embeddings server; set NOVA_EMBED_ENDPOINT and NOVA_EMBED_MODEL"]
    fn real_embedder_captures_semantics() {
        let endpoint = std::env::var("NOVA_EMBED_ENDPOINT").expect("NOVA_EMBED_ENDPOINT");
        let model = std::env::var("NOVA_EMBED_MODEL").unwrap_or_else(|_| "all-minilm".into());
        let e = RealEmbedder::new(&endpoint, &model).expect("connect to embeddings server");
        assert_eq!(e.dim(), 384);
        let a = e.embed("the quick brown fox jumps over the lazy dog");
        let b = e.embed("a speedy auburn canine leaps above a sluggish hound");
        let c = e.embed("the stock market rose three percent on Tuesday");
        let sim_ab = cosine_similarity(&a, &b);
        let sim_ac = cosine_similarity(&a, &c);
        assert!(sim_ab > sim_ac, "semantic neighbours should score higher");
    }
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
