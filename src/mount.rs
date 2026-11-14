//! What a host configures when it mounts the client: whose layers, who acts, which views.

use std::collections::BTreeSet;

use ikigai_core::{Error, Iri, Result};

/// The longest layer name a layer host accepts.
const LAYER_NAME_MAX: usize = 24;

/// One owner's set of layers. One set serves every site of that owner: the site is a field
/// of every entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layers {
    /// Inquiries for a person (tuplespace).
    pub inquiry: String,
    /// What the filter is confident is spam (tuplespace).
    pub spam: String,
    /// The borderline ones the holding stage checks (tuplespace).
    pub holding: String,
    /// Spam a person confirmed, which the holding stage compares with (channel).
    pub examples: String,
    /// What a person decided, for the stages that act on it to take: training and the
    /// edge's block set (tuplespace).
    pub intents: String,
}

impl Layers {
    /// The conventional names for `owner`: `inquiry-{owner}`, `spam-{owner}`,
    /// `holding-{owner}`, `examples-{owner}` and `intents-{owner}`.
    pub fn for_owner(owner: &str) -> Result<Self> {
        Self {
            inquiry: format!("inquiry-{owner}"),
            spam: format!("spam-{owner}"),
            holding: format!("holding-{owner}"),
            examples: format!("examples-{owner}"),
            intents: format!("intents-{owner}"),
        }
        .checked()
    }

    /// Refuses a name a layer host would refuse: lowercase `[a-z0-9-]`, at most 24
    /// characters.
    pub fn checked(self) -> Result<Self> {
        for name in [
            &self.inquiry,
            &self.spam,
            &self.holding,
            &self.examples,
            &self.intents,
        ] {
            let ok = !name.is_empty()
                && name.len() <= LAYER_NAME_MAX
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if !ok {
                return Err(Error::InvalidArgument {
                    name: "layers".to_string(),
                    detail: format!(
                        "`{name}` is not a layer name: lowercase [a-z0-9-], at most \
                         {LAYER_NAME_MAX} characters"
                    ),
                });
            }
        }
        Ok(self)
    }
}

/// A view the client can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum View {
    /// The pending inquiries, by site, newest first.
    Pending,
    /// One day's spam: arrivals by site and by what sent them there, with an excerpt each.
    Spam,
}

impl View {
    /// Every view.
    pub const ALL: [View; 2] = [View::Pending, View::Spam];
}

/// One mount of the client.
#[derive(Clone, Debug)]
pub struct Mount {
    pub(crate) layers: Layers,
    pub(crate) acting: Option<String>,
    pub(crate) views: BTreeSet<View>,
    pub(crate) action_base: String,
    pub(crate) embedder: Option<Iri>,
}

impl Mount {
    /// A mount over `layers`, showing every view, open to whoever the host lets in, posting
    /// its buttons to `/inquiry/action/{action}`.
    pub fn new(layers: Layers) -> Self {
        Self {
            layers,
            acting: None,
            views: View::ALL.into_iter().collect(),
            action_base: "/inquiry/action/".to_string(),
            embedder: None,
        }
    }

    /// Only `principal` may act or look through this mount; any other caller is Denied,
    /// whatever its grant holds. Every request still runs under the caller's own grant.
    pub fn acting_as(mut self, principal: impl Into<String>) -> Self {
        self.acting = Some(principal.into());
        self
    }

    /// Show only these views; the others are not bound.
    pub fn views(mut self, views: impl IntoIterator<Item = View>) -> Self {
        self.views = views.into_iter().collect();
        self
    }

    /// Where the host routes a POST to an action: a button posts to `{base}{action}`, and
    /// the host turns that into a Sink of `urn:iki:inquiry:action:{action}` with the form's
    /// `inquiry` field.
    pub fn action_base(mut self, base: impl Into<String>) -> Self {
        self.action_base = base.into();
        self
    }

    /// A local embedder: `confirm-spam` sources it with the text as `in` and keeps its
    /// answer with the example. Never a hosted model.
    pub fn embedder(mut self, iri: Iri) -> Self {
        self.embedder = Some(iri);
        self
    }

    /// The layers this mount works on.
    pub fn layers(&self) -> &Layers {
        &self.layers
    }
}
