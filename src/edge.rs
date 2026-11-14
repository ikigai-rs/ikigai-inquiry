//! What a host opens the edge half over: the classifier model, the routing policy table, the
//! intents layer, the sender-key secret and a store.

use std::sync::Arc;

use crate::block::BlockSet;
use crate::count::Counts;
use crate::endpoints::{MAX_BLOCK_DAYS, UNSCORED};
use crate::key::SenderKeys;
use crate::mount::Layers;
use crate::store::Store;

/// The largest inquiry, in bytes: the text plus every field. Past it the inquiry is refused,
/// never truncated. The classifier's own bound (ikigai-bayes's `MAX_MESSAGE_BYTES`).
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// The longest message, in characters: what the inquiry shapes admit as `inq:text`.
pub const MAX_TEXT_CHARS: usize = 10_000;

/// The longest name or organization, in characters: what the inquiry shapes admit.
pub const MAX_FIELD_CHARS: usize = 256;

/// The longest site name, in characters.
pub const MAX_SITE_CHARS: usize = 64;

/// The longest layer name, in characters (the layer host's own bound).
pub const MAX_LAYER_CHARS: usize = 24;

/// How long a block holds, in days, unless the host configures otherwise.
pub const DEFAULT_BLOCK_DAYS: u64 = 180;

fn check_layer(layer: &str) -> Result<(), String> {
    let ok = (1..=MAX_LAYER_CHARS).contains(&layer.len())
        && layer.starts_with(|c: char| c.is_ascii_lowercase())
        && layer
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "`{layer}` is not a layer name: 1 to {MAX_LAYER_CHARS} lowercase letters, digits \
             and hyphens, starting with a letter"
        ))
    }
}

/// Which layer each score class goes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePolicy {
    ham: String,
    unsure: String,
    spam: String,
}

impl RoutePolicy {
    /// `ham` to the first layer, `unsure` (and `unscored`) to the second, `spam` to the third.
    /// Each is a layer name: lowercase letters, digits and hyphens, at most 24 characters.
    pub fn new(
        ham: impl Into<String>,
        unsure: impl Into<String>,
        spam: impl Into<String>,
    ) -> Result<RoutePolicy, String> {
        let policy = RoutePolicy {
            ham: ham.into(),
            unsure: unsure.into(),
            spam: spam.into(),
        };
        for layer in policy.layers() {
            check_layer(layer)?;
        }
        Ok(policy)
    }

    /// The design's table over one owner's layers: ham to inquiry, unsure to holding, spam to
    /// spam.
    pub fn for_layers(layers: &Layers) -> Result<RoutePolicy, String> {
        RoutePolicy::new(&layers.inquiry, &layers.holding, &layers.spam)
    }

    /// The layer a score class goes to, or `None` for a class the policy does not know.
    pub fn layer_for(&self, class: &str) -> Option<&str> {
        match class {
            "ham" => Some(&self.ham),
            "unsure" | UNSCORED => Some(&self.unsure),
            "spam" => Some(&self.spam),
            _ => None,
        }
    }

    /// The policy's layers: ham's, unsure's, spam's.
    pub fn layers(&self) -> impl Iterator<Item = &str> {
        [&self.ham, &self.unsure, &self.spam]
            .into_iter()
            .map(String::as_str)
    }
}

/// What the host configures: the classifier model, the policy, and the intents layer apply
/// takes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The `ikigai-bayes` model scored at `urn:bayes:{model}:score`.
    pub model: String,
    /// Which layer each class goes to.
    pub policy: RoutePolicy,
    /// The intents layer: the airlock a person's decisions arrive through.
    pub intents: String,
    /// How long a block holds, in days.
    pub block_days: u64,
}

impl Config {
    /// A model over one owner's layers: the policy is [`RoutePolicy::for_layers`], the
    /// intents layer is theirs, and blocks hold [`DEFAULT_BLOCK_DAYS`].
    pub fn new(model: impl Into<String>, layers: &Layers) -> Result<Config, String> {
        Ok(Config {
            model: model.into(),
            policy: RoutePolicy::for_layers(layers)?,
            intents: layers.intents.clone(),
            block_days: DEFAULT_BLOCK_DAYS,
        })
    }
}

/// The edge half, as a host opens it.
#[derive(Debug)]
pub struct Edge {
    pub(crate) config: Config,
    pub(crate) keys: SenderKeys,
    pub(crate) blocks: BlockSet,
    pub(crate) counts: Counts,
}

impl Edge {
    /// The edge over `store` (where the block set and the counters are kept). A model name the
    /// classifier could not offer, a layer name a layer host would refuse, or a block set the
    /// store holds damaged, is refused.
    pub fn new(config: Config, keys: SenderKeys, store: Arc<dyn Store>) -> Result<Edge, String> {
        let model = &config.model;
        let ok = (1..=40).contains(&model.len())
            && model.starts_with(|c: char| c.is_ascii_lowercase())
            && model
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !ok {
            return Err(format!("`{model}` is not a classifier model name"));
        }
        check_layer(&config.intents)?;
        for layer in config.policy.layers() {
            check_layer(layer)?;
        }
        if !(1..=MAX_BLOCK_DAYS).contains(&config.block_days) {
            return Err(format!(
                "a block holds 1 to {MAX_BLOCK_DAYS} days, not {}",
                config.block_days
            ));
        }
        Ok(Edge {
            blocks: BlockSet::open(store.clone())?,
            counts: Counts::new(store),
            config,
            keys,
        })
    }

    /// The block set.
    pub fn blocks(&self) -> &BlockSet {
        &self.blocks
    }

    /// The counters.
    pub fn counts(&self) -> &Counts {
        &self.counts
    }
}
