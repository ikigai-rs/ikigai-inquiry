//! The confirmed spam examples: a CHANNEL layer of `inq:SpamExample` entries, read into
//! memory for each pass and compared by a direct scan. There is no vector database and no
//! index beside the channel: the channel is the store.
//!
//! The layers client's `confirm-spam` writes the entries ([`crate::layer::example`]), with
//! the embedding when its mount has an embedder. An entry without one is never compared.

use base64::Engine;
use ikigai_core::{Invocation, Result};
use serde_json::Value;

use crate::layer::{self, value};

/// One confirmed example. `text` is a stranger's words: compared and shown to the local model
/// as data, never interpreted, and never printed.
#[derive(Clone, PartialEq)]
pub struct SpamExample {
    /// The entry's IRI in the examples channel: the example's id.
    pub id: String,
    /// The inquiry it came from.
    pub inquiry: String,
    /// The site whose form it came from.
    pub site: String,
    /// What the visitor wrote.
    pub text: String,
    /// Its embedding.
    pub vector: Vec<f32>,
    /// The model that made the embedding, as the embedder named it.
    pub model: String,
    /// That model's digest.
    pub digest: String,
    /// A person's word for the kind of spam, if they gave one.
    pub label: Option<String>,
}

impl std::fmt::Debug for SpamExample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpamExample")
            .field("id", &self.id)
            .field("inquiry", &self.inquiry)
            .field("model", &self.model)
            .field("digest", &self.digest)
            .field("dimensions", &self.vector.len())
            .finish_non_exhaustive()
    }
}

impl SpamExample {
    /// The example a channel entry is, or `None` when the node is not one, or its embedding
    /// does not decode.
    pub fn from_node(node: &Value) -> Option<SpamExample> {
        if !layer::has_type(node, "SpamExample") {
            return None;
        }
        Some(SpamExample {
            id: node.get("@id")?.as_str()?.to_string(),
            inquiry: value(node, "inquiry")?,
            site: value(node, "site")?,
            text: value(node, "text")?,
            vector: decode(&value(node, "embedding")?)?,
            model: value(node, "embeddingModel")?,
            digest: value(node, "embeddingDigest")?,
            label: value(node, "label"),
        })
    }
}

/// An embedding as the channel stores it: base64 (standard alphabet, padded) of
/// little-endian f32s.
pub fn encode(vector: &[f32]) -> String {
    let bytes: Vec<u8> = vector.iter().flat_map(|x| x.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The inverse of [`encode`]; `None` for anything that is not whole f32s, or holds a value
/// that is not finite.
pub fn decode(text: &str) -> Option<Vec<f32>> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .ok()?;
    if bytes.is_empty() || bytes.len() % 4 != 0 {
        return None;
    }
    let vector: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect();
    vector.iter().all(|x| x.is_finite()).then_some(vector)
}

/// A model name as the embedder compares it: `embeddinggemma` is `embeddinggemma:latest`.
fn model_name(name: &str) -> String {
    let name = name.trim();
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{name}:latest")
    }
}

/// A digest as compared: lowercase, without a `sha256:` prefix.
fn digest(text: &str) -> String {
    let text = text.trim().to_ascii_lowercase();
    text.strip_prefix("sha256:").unwrap_or(&text).to_string()
}

/// Whether two vectors were made by the same model at the same digest: the only vectors that
/// are ever compared.
pub fn same_model(a_model: &str, a_digest: &str, b_model: &str, b_digest: &str) -> bool {
    model_name(a_model) == model_name(b_model) && digest(a_digest) == digest(b_digest)
}

/// The cosine of two vectors of one length; `None` when the lengths differ or either is zero.
pub fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let (mut dot, mut na, mut nb) = (0f64, 0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (f64::from(*x), f64::from(*y));
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return None;
    }
    Some((dot / (na.sqrt() * nb.sqrt())) as f32)
}

/// The examples made by `model` at `digest`, ranked by cosine with `vector`, closest first.
/// An example another model made, or another digest of this one, is never compared.
pub fn nearest<'a>(
    vector: &[f32],
    model: &str,
    digest: &str,
    examples: &'a [SpamExample],
) -> Vec<(f32, &'a SpamExample)> {
    let mut ranked: Vec<(f32, &SpamExample)> = examples
        .iter()
        .filter(|e| same_model(&e.model, &e.digest, model, digest))
        .filter_map(|e| cosine(vector, &e.vector).map(|score| (score, e)))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    ranked
}

/// Every example in the channel `layer`, read into memory.
pub async fn load(inv: &Invocation<'_>, layer: &str) -> Result<Vec<SpamExample>> {
    Ok(layer::entries(inv, layer)
        .await?
        .iter()
        .filter_map(SpamExample::from_node)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example(id: &str, vector: Vec<f32>, model: &str, digest: &str) -> SpamExample {
        SpamExample {
            id: id.into(),
            inquiry: id.into(),
            site: "a.example".into(),
            text: "t".into(),
            vector,
            model: model.into(),
            digest: digest.into(),
            label: None,
        }
    }

    #[test]
    fn embeddings_round_trip_as_little_endian_f32() {
        let v = vec![1.0f32, -0.5, 0.25];
        assert_eq!(decode(&encode(&v)), Some(v));
        // 1.0f32 little-endian is 00 00 80 3f.
        assert_eq!(encode(&[1.0]), "AACAPw==");
        assert_eq!(decode("AACA"), None, "three bytes are no f32");
        assert_eq!(decode(&encode(&[f32::NAN])), None);
        assert_eq!(decode("not base64!"), None);
    }

    #[test]
    fn cosine_is_the_angle() {
        assert_eq!(cosine(&[1.0, 0.0], &[2.0, 0.0]), Some(1.0));
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 3.0]), Some(0.0));
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0, 0.0]), None);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), None);
    }

    #[test]
    fn vectors_from_another_model_are_never_compared() {
        let examples = vec![
            example("same", vec![0.0, 1.0], "embeddinggemma:latest", "sha256:AB"),
            example("other-model", vec![1.0, 0.0], "nomic-embed-text", "ab"),
            example("other-digest", vec![1.0, 0.0], "embeddinggemma", "cd"),
        ];
        // The other two are identical to the query; only the same model's vector is ranked.
        let ranked = nearest(&[1.0, 0.0], "embeddinggemma", "ab", &examples);
        let ids: Vec<&str> = ranked.iter().map(|(_, e)| e.id.as_str()).collect();
        assert_eq!(ids, ["same"]);
        assert_eq!(ranked[0].0, 0.0);
    }
}
