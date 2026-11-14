//! The funnel a holding inquiry goes through: the confirmed examples first, then a local
//! model, and on any failure, on to a person.
//!
//! 1. **Embed** the text with `urn:llm:embed` and the policy's model: the raw text, with no
//!    prompt prefix. The answer names the model and its digest.
//! 2. **Nearest**: the cosine with every confirmed example the SAME model made at the same
//!    digest, by a direct scan ([`example::nearest`]). At or above the policy's threshold,
//!    the inquiry is spam, and the example's id and the score are recorded.
//! 3. Otherwise **ask the local model** through the classify contract,
//!    `urn:iki:classify:{classifier}:answer` (ikigai-classify), with the text as `content`
//!    (data, never part of an instruction) and the closest few examples as `context`, for a
//!    label of the closed set `spam`, `not-spam`. Any other answer, an abstention included,
//!    sends the inquiry to a person.
//!
//! Any failure (the embedder or the model server down, a timeout, an answer that does not
//! read) sends the inquiry to the inquiry layer, marked `unjudged`: never back into holding,
//! and never dropped.

use ikigai_core::{Error, Invocation, Iri, Result, Time, Verb};
use serde_json::{json, Value};

use crate::example::{self, SpamExample};
use crate::layer::{iri, json_answer, request};
use crate::policy::Policy;

/// The embedder the funnel asks.
pub const EMBED: &str = "urn:llm:embed";

/// The labels the classifier is asked to choose from.
pub const LABELS: [&str; 2] = ["spam", "not-spam"];

/// Where the funnel sends an inquiry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The spam layer.
    Spam,
    /// The inquiry layer, for a person.
    Inquiry,
}

impl Verdict {
    /// `spam` or `inquiry`, as `inq:verdict` records it.
    pub fn word(self) -> &'static str {
        match self {
            Verdict::Spam => "spam",
            Verdict::Inquiry => "inquiry",
        }
    }
}

/// Why the funnel decided what it did.
#[derive(Clone, Debug, PartialEq)]
pub enum Reason {
    /// A confirmed example is at or above the threshold.
    Example {
        /// The example's id: its entry IRI in the examples channel.
        id: String,
        /// The cosine.
        score: f32,
    },
    /// The local model answered `spam` or `not-spam`.
    Model,
    /// The local model answered something outside the closed set.
    Abstained,
    /// The funnel could not judge: `stage` (`embed`, `examples` or `model`) failed.
    Unjudged {
        /// The step that failed.
        stage: &'static str,
    },
}

impl Reason {
    /// The short word `inq:reason` records.
    pub fn word(&self) -> &'static str {
        match self {
            Reason::Example { .. } => "example",
            Reason::Model => "model",
            Reason::Abstained => "abstained",
            Reason::Unjudged { .. } => "unjudged",
        }
    }
}

/// What the funnel decided, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Judgment {
    /// Where it goes.
    pub verdict: Verdict,
    /// Why.
    pub reason: Reason,
}

impl Judgment {
    fn unjudged(stage: &'static str) -> Judgment {
        Judgment {
            verdict: Verdict::Inquiry,
            reason: Reason::Unjudged { stage },
        }
    }
}

/// What `urn:llm:embed` answered: the vector, and the model and digest that made it.
#[derive(Clone, Debug, PartialEq)]
pub struct Embedding {
    /// The vector.
    pub vector: Vec<f32>,
    /// The installed model's name.
    pub model: String,
    /// Its digest.
    pub digest: String,
}

/// The policy's deadline for one embedder or classifier call, when the kernel has a clock.
fn deadline(inv: &Invocation<'_>, policy: &Policy) -> Option<Time> {
    let ms = u64::try_from(policy.timeout.as_millis()).unwrap_or(u64::MAX);
    inv.now()
        .map(|now| Time::from_millis(now.as_millis().saturating_add(ms)))
}

/// Issue `request` within the policy's deadline.
async fn issue_bounded(
    inv: &Invocation<'_>,
    policy: &Policy,
    request: ikigai_core::Request,
) -> Result<ikigai_core::Representation> {
    match deadline(inv, policy) {
        Some(at) => inv.with_not_after(at).issue(request).await,
        None => inv.issue(request).await,
    }
}

/// Embed `text` with the policy's model: the raw text, no prompt prefix.
pub async fn embed(inv: &Invocation<'_>, policy: &Policy, text: &str) -> Result<Embedding> {
    let deadline = deadline(inv, policy);
    embed_at(inv, &iri(EMBED)?, &policy.embed_model, text, deadline).await
}

/// Embed `text` with `model` at `embedder`, a Source of the `urn:llm:embed` contract: the raw
/// text, no prompt prefix, answered as JSON naming the vector, the model and its digest.
/// Within `deadline` when there is one. The layers client embeds a confirmed example this
/// way, so the holding stage can compare with it.
pub async fn embed_at(
    inv: &Invocation<'_>,
    embedder: &Iri,
    model: &str,
    text: &str,
    deadline: Option<Time>,
) -> Result<Embedding> {
    let request = request(
        Verb::Source,
        embedder.clone(),
        &[("model", model), ("text", text), ("as", "application/json")],
    );
    let answer = match deadline {
        Some(at) => inv.with_not_after(at).issue(request).await?,
        None => inv.issue(request).await?,
    };
    let what = embedder.as_str();
    let doc = json_answer(&answer.bytes, what)?;
    let vector: Option<Vec<f32>> = doc["vector"].as_array().and_then(|xs| {
        xs.iter()
            .map(|x| x.as_f64().map(|x| x as f32).filter(|x| x.is_finite()))
            .collect()
    });
    match (vector, doc["model"].as_str(), doc["digest"].as_str()) {
        (Some(vector), Some(model), Some(digest))
            if !vector.is_empty() && !model.is_empty() && !digest.is_empty() =>
        {
            Ok(Embedding {
                vector,
                model: model.into(),
                digest: digest.into(),
            })
        }
        _ => Err(Error::Endpoint(format!(
            "{what} answered no vector, model and digest: a vector is compared only with one \
             the same model and digest made"
        ))),
    }
}

/// Ask the policy's classifier about `text`, with `closest` as context: its label.
async fn classify(
    inv: &Invocation<'_>,
    policy: &Policy,
    text: &str,
    closest: &[(f32, &SpamExample)],
) -> Result<Option<String>> {
    let target = format!("urn:iki:classify:{}:answer", policy.classifier);
    let context: Vec<Value> = closest
        .iter()
        .map(|(score, e)| {
            json!({ "label": "spam", "kind": e.label, "score": score, "content": e.text })
        })
        .collect();
    let answer = issue_bounded(
        inv,
        policy,
        request(
            Verb::Source,
            iri(&target)?,
            &[
                ("content", text),
                ("labels", &LABELS.join(" ")),
                ("context", &Value::Array(context).to_string()),
                ("as", "application/json"),
            ],
        ),
    )
    .await?;
    let doc = json_answer(&answer.bytes, &target)?;
    Ok(doc["label"].as_str().map(str::to_string))
}

/// Judge `text` against `examples` (already read from the channel; `Err` when that read
/// failed). Never fails: a failure is a judgment, [`Reason::Unjudged`].
pub async fn judge(
    inv: &Invocation<'_>,
    policy: &Policy,
    text: &str,
    examples: &Result<Vec<SpamExample>>,
) -> Judgment {
    let Ok(embedding) = embed(inv, policy, text).await else {
        return Judgment::unjudged("embed");
    };
    let Ok(examples) = examples else {
        return Judgment::unjudged("examples");
    };
    let ranked = example::nearest(
        &embedding.vector,
        &embedding.model,
        &embedding.digest,
        examples,
    );
    if let Some((score, closest)) = ranked.first() {
        if *score >= policy.threshold {
            return Judgment {
                verdict: Verdict::Spam,
                reason: Reason::Example {
                    id: closest.id.clone(),
                    score: *score,
                },
            };
        }
    }
    let few = &ranked[..ranked.len().min(policy.context_examples)];
    match classify(inv, policy, text, few).await {
        Err(_) => Judgment::unjudged("model"),
        Ok(Some(label)) if label == "spam" => Judgment {
            verdict: Verdict::Spam,
            reason: Reason::Model,
        },
        Ok(Some(label)) if label == "not-spam" => Judgment {
            verdict: Verdict::Inquiry,
            reason: Reason::Model,
        },
        Ok(_) => Judgment {
            verdict: Verdict::Inquiry,
            reason: Reason::Abstained,
        },
    }
}
