//! The actions: each a Sink, `urn:iki:inquiry:action:name:{action} inquiry=<id>`, run under the
//! caller's own grant.
//!
//! A move is OUT the new copy, THEN TAKE the old one, so a failure between the two leaves the
//! inquiry in both layers, never in neither. A copy's name is fixed by the copy it came from,
//! so a retried action writes the same tuple again (a replay the layer answers the same) and
//! its take is the layer's Conflict. An action on an inquiry another action already took is a
//! Conflict naming who took it, before anything is written. An intent is written only after
//! its take succeeded, so a lost race trains nothing.

use std::sync::Arc;

use async_trait::async_trait;
use ikigai_core::{
    ArgSpec, Description, Endpoint, Error, Invocation, ReprType, Representation, Result, Verb,
};

use crate::funnel::{self, Embedding};
use crate::layer::{self, Inquiry};
use crate::mount::Mount;
use crate::vocab::is_inquiry_id;

/// One action a person can take on an inquiry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Take it from spam: no training, no example.
    Remove,
    /// Out to inquiry, take from spam, and an intent to train it as ham.
    NotSpam,
    /// Add it to the examples channel (embedded, when the mount has an embedder), take it
    /// from spam, and an intent to train it as spam.
    ConfirmSpam,
    /// Out to spam, take from inquiry, and an intent to train it as spam.
    FlagSpam,
    /// An intent for the edge to block its sender.
    Block,
    /// An intent for the edge to unblock its sender.
    Unblock,
    /// Mark it handled: take it from inquiry. The view offers the `mailto:` link to write
    /// the reply itself.
    Respond,
}

impl Action {
    /// Every action.
    pub const ALL: [Action; 7] = [
        Action::Remove,
        Action::NotSpam,
        Action::ConfirmSpam,
        Action::FlagSpam,
        Action::Block,
        Action::Unblock,
        Action::Respond,
    ];

    /// The action's name: the last segment of its IRI and of the path a button posts to.
    pub fn name(self) -> &'static str {
        match self {
            Action::Remove => "remove",
            Action::NotSpam => "not-spam",
            Action::ConfirmSpam => "confirm-spam",
            Action::FlagSpam => "flag-spam",
            Action::Block => "block",
            Action::Unblock => "unblock",
            Action::Respond => "respond",
        }
    }

    /// `urn:iki:inquiry:action:name:{name}`.
    pub fn iri(self) -> String {
        format!("urn:iki:inquiry:action:name:{}", self.name())
    }

    fn summary(self) -> &'static str {
        match self {
            Action::Remove => "Takes the inquiry from spam, with no training and no example.",
            Action::NotSpam => {
                "Moves the inquiry from spam to inquiry (out the copy, then take the old), and \
                 writes an intent to train it as ham."
            }
            Action::ConfirmSpam => {
                "Adds the inquiry to the examples channel, embedded by the mount's local \
                 embedder when it has one, takes it from spam, and writes an intent to train it \
                 as spam."
            }
            Action::FlagSpam => {
                "Moves the inquiry from inquiry to spam (out the copy, then take the old), and \
                 writes an intent to train it as spam."
            }
            Action::Block => "Writes an intent for the edge to block the inquiry's sender.",
            Action::Unblock => "Writes an intent for the edge to unblock the inquiry's sender.",
            Action::Respond => {
                "Marks the inquiry handled by taking it from inquiry; the reply itself is \
                 written from the view's mailto: link."
            }
        }
    }
}

/// The endpoint of one action under one mount.
pub struct ActionEndpoint {
    action: Action,
    mount: Arc<Mount>,
}

impl ActionEndpoint {
    pub(crate) fn new(action: Action, mount: Arc<Mount>) -> Self {
        Self { action, mount }
    }
}

/// Refuses a request in any verb but `verb`, then a caller other than the mount's acting
/// identity, when it names one.
pub(crate) fn check_acting(mount: &Mount, inv: &Invocation<'_>, verb: Verb) -> Result<()> {
    if inv.request.verb != verb {
        return Err(Error::Unsupported {
            verb: Some(inv.request.verb),
            detail: format!("`{}` answers {verb:?} only", inv.request.target.as_str()),
        });
    }
    match &mount.acting {
        Some(acting) if inv.capability.principal() != Some(acting.as_str()) => Err(Error::Denied(
            format!("this inquiry client acts only as {acting}"),
        )),
        _ => Ok(()),
    }
}

fn done(message: String) -> Representation {
    Representation::new(ReprType::new("text/plain"), message.into_bytes())
}

/// The pending copy of `id` in `layer`, or why there is none.
async fn pending(inv: &Invocation<'_>, layer: &str, id: &str) -> Result<Inquiry> {
    match layer::find(inv, layer, id).await? {
        Some(found) => Ok(found),
        None => Err(layer::absent(inv, layer, id).await),
    }
}

/// OUT the copy, THEN TAKE the old.
async fn moved(
    inv: &Invocation<'_>,
    from: &str,
    to: &str,
    reason: &str,
    id: &str,
) -> Result<Inquiry> {
    let old = pending(inv, from, id).await?;
    layer::out(inv, to, &layer::copy(to, &old, reason)).await?;
    layer::take(inv, from, &old.subject).await?;
    Ok(old)
}

/// A train intent, named by the copy it is about, so a retry writes the same tuple again.
async fn train(inv: &Invocation<'_>, mount: &Mount, intent: &str, about: &Inquiry) -> Result<()> {
    let suffix = format!("m{}", about.seq);
    let tuple = layer::intent(
        &mount.layers.intents,
        intent,
        &about.id,
        &suffix,
        Some(about),
        true,
    );
    layer::out(inv, &mount.layers.intents, &tuple).await
}

/// A block or unblock intent: named by the kernel clock's time, since a sender may be
/// blocked and unblocked again.
async fn block(inv: &Invocation<'_>, mount: &Mount, intent: &str, id: &str) -> Result<()> {
    let now = inv.now().ok_or_else(|| {
        Error::Unavailable("the kernel has no clock, so an intent cannot be named".to_string())
    })?;
    let layers = &mount.layers;
    let mut about = None;
    for name in [&layers.inquiry, &layers.spam, &layers.holding] {
        if let Some(found) = layer::find(inv, name, id).await? {
            about = Some(found);
            break;
        }
    }
    let suffix = format!("t{}", now.as_millis());
    let tuple = layer::intent(&layers.intents, intent, id, &suffix, about.as_ref(), false);
    layer::out(inv, &layers.intents, &tuple).await
}

/// The example's embedding, when the mount has an embedder. A failing embedder fails the
/// action before anything is written, so a person can try again.
async fn embed(inv: &Invocation<'_>, mount: &Mount, text: &str) -> Result<Option<Embedding>> {
    let Some(embedder) = &mount.embedder else {
        return Ok(None);
    };
    funnel::embed_at(inv, embedder, &mount.embed_model, text, None)
        .await
        .map(Some)
}

#[async_trait]
impl Endpoint for ActionEndpoint {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        check_acting(&self.mount, inv, Verb::Sink)?;
        let id = inv.inline_str("inquiry")?;
        if !is_inquiry_id(id) {
            return Err(Error::InvalidArgument {
                name: "inquiry".to_string(),
                detail: "not an inquiry id: a letter or digit, then up to 63 letters, digits, \
                         dots, underscores or hyphens"
                    .to_string(),
            });
        }
        let mount = &self.mount;
        let layers = &mount.layers;
        let message = match self.action {
            Action::Remove => {
                let old = pending(inv, &layers.spam, id).await?;
                layer::take(inv, &layers.spam, &old.subject).await?;
                format!("Removed inquiry {id} from spam.")
            }
            Action::NotSpam => {
                let old = moved(inv, &layers.spam, &layers.inquiry, "not-spam", id).await?;
                train(inv, mount, "train-ham", &old).await?;
                format!("Moved inquiry {id} from spam to your inquiries.")
            }
            Action::ConfirmSpam => {
                let old = pending(inv, &layers.spam, id).await?;
                let embedding = embed(inv, mount, &old.text).await?;
                layer::say(
                    inv,
                    &layers.examples,
                    &layer::example(&old, embedding.as_ref()),
                )
                .await?;
                layer::take(inv, &layers.spam, &old.subject).await?;
                train(inv, mount, "train-spam", &old).await?;
                format!("Confirmed inquiry {id} as spam and kept it as an example.")
            }
            Action::FlagSpam => {
                let old = moved(inv, &layers.inquiry, &layers.spam, "flagged", id).await?;
                train(inv, mount, "train-spam", &old).await?;
                format!("Moved inquiry {id} to spam.")
            }
            Action::Block => {
                block(inv, mount, "block", id).await?;
                format!("Asked to block the sender of inquiry {id}.")
            }
            Action::Unblock => {
                block(inv, mount, "unblock", id).await?;
                format!("Asked to unblock the sender of inquiry {id}.")
            }
            Action::Respond => {
                let old = pending(inv, &layers.inquiry, id).await?;
                layer::take(inv, &layers.inquiry, &old.subject).await?;
                format!("Marked inquiry {id} handled.")
            }
        };
        Ok(done(message))
    }

    fn name(&self) -> &str {
        match self.action {
            Action::Remove => "inquiry-remove",
            Action::NotSpam => "inquiry-not-spam",
            Action::ConfirmSpam => "inquiry-confirm-spam",
            Action::FlagSpam => "inquiry-flag-spam",
            Action::Block => "inquiry-block",
            Action::Unblock => "inquiry-unblock",
            Action::Respond => "inquiry-respond",
        }
    }

    fn describe(&self) -> Description {
        Description::new(self.name())
            .title(format!("Inquiry: {}", self.action.name()))
            .summary(self.action.summary())
            .verb(Verb::Sink)
            .input(ArgSpec::new("inquiry").summary("the inquiry's id"))
            .output("text/plain")
    }
}
