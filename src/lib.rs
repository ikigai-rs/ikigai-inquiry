//! `ikigai-inquiry`: inquiries from public contact forms, as resources.
//!
//! The module will bind, under `urn:iki:inquiry:*`:
//!
//! - the edge half: a router that scores a submitted inquiry and puts it into one layer
//!   (holding, spam or inquiry), a block set keyed by a hash of the sender, and aggregate
//!   day counters (not built yet);
//! - a holding stage that takes each borderline inquiry once, compares it with confirmed spam
//!   examples, asks a local model only when no example is close, and sends it on (not built
//!   yet);
//! - a layers client: views and actions that any host mounts. **Built**: [`space`].
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
//! | `urn:iki:inquiry:view:pending` | Source | the pending inquiries, by site, newest first (HTML) |
//! | `urn:iki:inquiry:view:spam` | Source | one day's spam, `day=YYYY-MM-DD` or today (HTML) |
//! | `urn:iki:inquiry:action:{action}` | Sink | one [`Action`] on `inquiry=<id>` (text) |
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
pub mod layer;
pub mod mount;
pub mod text;
pub mod view;
pub mod vocab;

pub use action::Action;
pub use mount::{Layers, Mount, View};

/// The pending inquiries view.
pub const PENDING_VIEW: &str = "urn:iki:inquiry:view:pending";
/// The day's spam view.
pub const SPAM_VIEW: &str = "urn:iki:inquiry:view:spam";

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
                template("urn:iki:inquiry:data:pending:{key}"),
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
                template("urn:iki:inquiry:data:spam:{day}:{part}"),
                view::SpamData(mount.clone()),
            );
    }
    space
}

fn template(pattern: &str) -> UriTemplate {
    UriTemplate::parse(pattern).expect("a fixed URI template")
}
