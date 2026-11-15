//! What one owner's holding stage is configured with: the host's policy row.

use std::time::Duration;

/// The embedding model the design names: `embeddinggemma`, given raw text with no prompt
/// prefix.
pub const DEFAULT_EMBED_MODEL: &str = "embeddinggemma";

/// The cosine at or above which a confirmed example decides: 0.85.
pub const DEFAULT_THRESHOLD: f32 = 0.85;

/// How many of the closest examples the model is shown as context.
pub const DEFAULT_CONTEXT_EXAMPLES: usize = 3;

/// How long the embedder or the model may take before the inquiry is sent on unjudged.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// One owner's holding stage: its layers, its funnel, and whether it runs in shadow.
///
/// Nothing here names a site or a host; the site is a field of every inquiry.
#[derive(Clone, Debug, PartialEq)]
pub struct Policy {
    /// The tuplespace the edge puts borderline inquiries in.
    pub holding: String,
    /// The tuplespace spam is sent to.
    pub spam: String,
    /// The tuplespace inquiries for a person are sent to.
    pub inquiry: String,
    /// The channel of confirmed `inq:SpamExample` entries.
    pub examples: String,
    /// The model `urn:llm:embed` is asked for. Vectors are compared only with vectors this
    /// model made, at the digest the embedder answers.
    pub embed_model: String,
    /// The cosine at or above which the nearest example decides spam.
    pub threshold: f32,
    /// The classifier asked when no example is close: `urn:iki:classify:{classifier}:answer`,
    /// a local model.
    pub classifier: String,
    /// How many of the closest examples the classifier is shown as context.
    pub context_examples: usize,
    /// The deadline of each embedder and classifier call.
    pub timeout: Duration,
    /// Shadow: record what the funnel would do, and send every holding inquiry to the
    /// inquiry layer.
    pub shadow: bool,
}

impl Policy {
    /// The layers of owner `owner` (`inquiry-hold-{owner}`, `inquiry-spam-{owner}`, `inquiry-{owner}`,
    /// `inquiry-example-{owner}`), the classifier `inquiry-{owner}`, and the design's defaults.
    /// Refused when `owner` would not make a layer name: lowercase ASCII letters, digits and
    /// `-`, starting with a letter or digit, at most 12 characters. Layer names use the
    /// same shared prefix and length checks as [`crate::Layers::for_owner`].
    pub fn for_owner(owner: &str) -> Result<Policy, String> {
        let layers = crate::Layers::for_owner(owner).map_err(|error| error.to_string())?;
        Ok(Policy {
            holding: layers.holding,
            spam: layers.spam,
            inquiry: layers.inquiry,
            examples: layers.examples,
            embed_model: DEFAULT_EMBED_MODEL.into(),
            threshold: DEFAULT_THRESHOLD,
            classifier: format!("inquiry-{owner}"),
            context_examples: DEFAULT_CONTEXT_EXAMPLES,
            timeout: DEFAULT_TIMEOUT,
            shadow: false,
        })
    }

    /// The same policy in shadow (builder).
    pub fn shadow(mut self, shadow: bool) -> Self {
        self.shadow = shadow;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owner_names_its_layers() {
        let p = Policy::for_owner("b").unwrap();
        assert_eq!(
            (&*p.holding, &*p.spam, &*p.inquiry, &*p.examples),
            (
                "inquiry-hold-b",
                "inquiry-spam-b",
                "inquiry-b",
                "inquiry-example-b"
            )
        );
        assert_eq!(p.embed_model, "embeddinggemma");
        assert_eq!(p.threshold, 0.85);
        assert!(!p.shadow);
        for bad in ["", "-b", "B", "a:b", "a b", &"a".repeat(49)] {
            assert!(Policy::for_owner(bad).is_err(), "{bad}");
        }
    }
}
