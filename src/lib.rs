//! `ikigai-inquiry`: inquiries from public contact forms, as resources.
//!
//! The module binds, under `urn:iki:inquiry:*`:
//!
//! - the edge half: a router that scores a submitted inquiry and puts it into one layer
//!   (holding, spam or inquiry), a block set keyed by a hash of the sender, the airlock's
//!   handler for what people decided, and aggregate day counters. **Built**: [`edge_space`];
//! - a holding stage that takes each borderline inquiry once, compares it with confirmed spam
//!   examples, asks a local model only when no example is close, and sends it on. **Built**:
//!   [`holding_space`];
//! - a layers client: views and actions that any host mounts. **Built**: [`space`].
//!
//! # The edge half
//!
//! What runs where a contact form posts. A host opens an [`Edge`] over its sender-key secret
//! ([`SenderKeys`], 32 bytes read from a file), a [`Store`] and a [`Config`] (the classifier
//! model, the [`RoutePolicy`] table and the intents layer, all from one owner's [`Layers`]), and
//! mounts [`edge_space`]:
//!
//! | resource | verb | what it does | capability |
//! |---|---|---|---|
//! | `urn:iki:inquiry:route` | Sink | scores an inquiry and drops ONE `inq:Inquiry` tuple into the layer the policy picks | `urn:cap:inquiry:route` |
//! | `urn:iki:inquiry:sender-key` | Source | the key of an `address=` or a `domain=` | `urn:cap:inquiry:sender-key` |
//! | `urn:iki:inquiry:block:key:{key}` | Exists | the door's check: a block holds now, or `NotFound` | `urn:cap:inquiry:block:check` |
//! | `urn:iki:inquiry:block:key:{key}` | Source | the block's record | `urn:cap:inquiry:block:read` |
//! | `urn:iki:inquiry:block:key:{key}` | Delete | lifts the block | `urn:cap:inquiry:block:lift` |
//! | `urn:iki:inquiry:apply` | Sink | the airlock's handler: applies, then takes, the pending intents | `urn:cap:inquiry:apply` |
//! | `urn:iki:inquiry:count:date:{date}` | Source | per site: inquiries routed to each layer, and blocked drops | `urn:cap:inquiry:count:read` |
//! | `urn:iki:inquiry:count:kind:confusion:date:{date}` | Source | score class × a person's label | `urn:cap:inquiry:count:read` |
//!
//! The router scores the inquiry through the kernel at `urn:bayes:{model}:score` (an
//! `ikigai-bayes` model the host mounts) and maps its class through the [`RoutePolicy`]: `ham` to
//! the inquiry layer, `unsure` to holding, `spam` to spam. It then `out`s one tuple to
//! `urn:layer:{layer}:tuples`, written by oxttl ([`rdf`]) in the terms the shipped inquiry
//! shapes admit: `inq:inquiry` (the id, stable across layers), `inq:site`, `inq:text`,
//! `inq:replyTo`, `inq:name`, `inq:organization`, `inq:sender` (the address key),
//! `inq:received`, `inq:reason`, and the verdict so far as `inq:class` and `inq:classScore`
//! (`inq:verdict` and `inq:score` are the holding stage's, when it sends one on). A sender whose address key or
//! domain key is blocked is dropped before scoring and counted; nothing of the inquiry is
//! kept. A classifier that cannot answer does not lose the inquiry: it goes where an unsure
//! one goes, with the verdict `unscored`.
//!
//! Apply is the other side of the airlock the layers client writes into: it reads the
//! intents layer, and for each intent trains or untrains the classifier from the inquiry's
//! layer copy, or writes or lifts a block, then takes the intent.
//!
//! The router and apply reach the classifier and the layers under their caller's capability:
//! the router's grant holds the model's read capability and each policy layer's `out`; the
//! airlock's grant holds the model's train capability, each policy layer's `read`, and `read`
//! and `take` on the intents layer.
//!
//! Inquiry text is a stranger's text: data, never instructions. The edge never interprets it,
//! sends it only to the classifier and the layers the host mounts, and keeps none of it: the
//! block set holds keys, the counters hold numbers.
//!
//! ```
//! use std::sync::Arc;
//! use ikigai_inquiry::{Config, Edge, Layers, MemoryStore, SenderKeys};
//!
//! let config = Config::new("contact", &Layers::for_owner("brian").unwrap()).unwrap();
//! let edge = Edge::new(config, SenderKeys::new([7; 32]), Arc::new(MemoryStore::new())).unwrap();
//! let space = ikigai_inquiry::edge_space(&Arc::new(edge));
//! # let _ = space;
//! ```
//!
//! # The holding stage
//!
//! | resource | verb | what it is |
//! | --- | --- | --- |
//! | `urn:iki:inquiry:stage:name:holding` | Sink | one pass, `min-age=` seconds (text) |
//!
//! One set of layers per owner ([`Policy::for_owner`]), the site a field of every inquiry. A
//! host binds [`holding_space`] over the owner's [`Policy`] and writes to the stage on a
//! schedule, with `min-age=` from its policy row, under a worker's own grant. Two workers may
//! run at once: each tuple is taken once, by its subject, and the other is answered
//! `Conflict` and moves on. [`holding`] says what one pass does, [`funnel`] how an inquiry is
//! judged, and [`example`] what the confirmed-spam store is.
//!
//! Inquiry text is a stranger's text: data, never instructions. The stage never interprets
//! it, sends it only to the layers, the configured embedder and the configured local
//! classifier, and never puts it in an answer or an error. The layers seal it at rest, and the
//! stage stores nothing itself: the confirmed examples are read into memory for each pass.
//!
//! # The layers client
//!
//! One set of layers per owner ([`Layers`]), the site a field of every inquiry. A host mounts
//! the client with [`space`] over a [`Mount`] (whose layers, which identity acts, which
//! views show) and adds only routing and sign-in: it serves a view at a path, turns a POST of
//! a view's form into a Sink of the action, and mints the signed-in person's capability.
//!
//! | resource | verb | what it is |
//! | --- | --- | --- |
//! | `urn:iki:inquiry:view:name:pending` | Source | the pending inquiries, by site, newest first (HTML) |
//! | `urn:iki:inquiry:view:name:spam` | Source | one day's spam, `day=YYYY-MM-DD` or today (HTML) |
//! | `urn:iki:inquiry:action:name:{action}` | Sink | one [`Action`] on `inquiry=<id>` (text) |
//!
//! Every request runs under the caller's own grant: the client holds no authority of its
//! own, so what a person can do through it is what their layer grants allow. A person who
//! uses every view and action holds, on the mount's layers: `read`, `out` and `take` on
//! inquiry and spam, `read` on holding, `say` on examples, and `out` on intents.
//!
//! Inquiry text is a stranger's text: data, never instructions. The client never interprets
//! it, sends it only to the layers and the mount's local embedder, and puts it on a page only
//! through `$h{…}` ([`view`] says how). The layers seal it at rest.
//!
//! The vocabulary, and the context and shapes a layer host opens to admit it, are in
//! [`vocab`]. Nothing here names a site, an owner or a host. The design is ledger #3680.

#![forbid(unsafe_code)]

use std::sync::Arc;

use ikigai_core::{EndpointSpace, Exact, UriTemplate};

pub mod action;
mod block;
mod count;
pub mod date;
mod edge;
mod endpoints;
pub mod example;
pub mod funnel;
pub mod holding;
mod key;
pub mod layer;
pub mod mount;
pub mod policy;
pub mod rdf;
mod store;
pub mod text;
mod time;
pub mod view;
pub mod vocab;

pub use action::Action;
pub use block::{Block, BlockSet, MAX_BLOCKS, MAX_LABEL_CHARS};
pub use count::{Counts, Day, MAX_SITES_PER_DAY, OTHER_SITE};
pub use edge::{
    Config, Edge, RoutePolicy, DEFAULT_BLOCK_DAYS, MAX_FIELD_CHARS, MAX_LAYER_CHARS,
    MAX_MESSAGE_BYTES, MAX_SITE_CHARS, MAX_TEXT_CHARS,
};
pub use endpoints::*;
pub use example::SpamExample;
pub use funnel::{Judgment, Reason, Verdict};
pub use holding::{HoldingStage, HOLDING_STAGE};
pub use key::{Key, Scope, SenderKeys, MAX_ADDRESS_BYTES, SECRET_BYTES};
pub use mount::{Layers, Mount, View};
pub use policy::Policy;
#[cfg(not(target_family = "wasm"))]
pub use store::DirStore;
pub use store::{MemoryStore, Store};

/// The pending inquiries view.
pub const PENDING_VIEW: &str = "urn:iki:inquiry:view:name:pending";
/// The day's spam view.
pub const SPAM_VIEW: &str = "urn:iki:inquiry:view:name:spam";

/// The layers client for one mount: its views, its actions, and the templates and data
/// they are filled from.
pub fn space(mount: Mount) -> EndpointSpace {
    let mount = Arc::new(mount);
    let mut space = EndpointSpace::new().bind(Exact::new(view::EACH), ikigai_fn::each());
    for action in Action::ALL {
        space = space.bind(
            Exact::new(action.iri()),
            action::ActionEndpoint::new(action, mount.clone()),
        );
    }
    for (iri, name, text) in view::templates(&mount) {
        space = space.bind(Exact::new(iri), view::template(name, text));
    }
    let named = |iri: &str| ikigai_core::Iri::parse(iri).expect("a fixed IRI");
    if mount.views.contains(&View::Pending) {
        space = space
            .bind(Exact::new(PENDING_VIEW), view::PendingView(mount.clone()))
            .bind(
                Exact::new(view::PENDING_SECTION),
                ikigai_fn::compose_over(named(view::PENDING_SECTION_TEMPLATE))
                    .named("inquiry-pending-section"),
            )
            .bind(
                template("urn:iki:inquiry:data:pending:site:{key}"),
                view::PendingData(mount.clone()),
            );
    }
    if mount.views.contains(&View::Spam) {
        space = space
            .bind(Exact::new(SPAM_VIEW), view::SpamView(mount.clone()))
            .bind(
                Exact::new(view::SPAM_BODY),
                ikigai_fn::compose_over(named(view::SPAM_BODY_TEMPLATE)).named("inquiry-spam-body"),
            )
            .bind(
                template("urn:iki:inquiry:data:spam:date:{day}:part:{part}"),
                view::SpamData(mount.clone()),
            );
    }
    space
}

/// The holding stage of one owner, bound at [`HOLDING_STAGE`]. Built from the host's policy,
/// so the host names the space.
pub fn holding_space(policy: Policy) -> EndpointSpace {
    EndpointSpace::new().bind(Exact::new(HOLDING_STAGE), HoldingStage::new(policy))
}

fn template(pattern: &str) -> UriTemplate {
    UriTemplate::parse(pattern).expect("a fixed URI template")
}
