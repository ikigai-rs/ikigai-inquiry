//! The doors: the router, the sender-key door, the block set, the airlock's apply and the day
//! counters. Each declares the capability the kernel enforces for it; the router and apply
//! reach the classifier and the layers through the kernel, under the caller's capability.

use std::sync::Arc;

use async_trait::async_trait;
use ikigai_core::{
    ActionSpec, ArgRef, ArgSpec, Description, Endpoint, EndpointSpace, Error, Exact, Invocation,
    Iri, ReprType, Representation, Request, Result, Time, UriTemplate, Verb,
};
use serde_json::{json, Value};

use crate::block::{check_label, Block};
use crate::count::Tally;
use crate::edge::{Edge, MAX_FIELD_CHARS, MAX_MESSAGE_BYTES, MAX_SITE_CHARS, MAX_TEXT_CHARS};
use crate::key::{Key, Scope};
use crate::layer::{self, Intent};
use crate::rdf::{self, Arrival};
use crate::time;

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const TEXT: &str = "text/plain";
const JSON: &str = "application/json";
// Every `json!` object below lists its keys in sorted order, so a JSON face is byte-identical
// whether or not some other crate in a host's graph turns on serde_json's `preserve_order`.
const FACES: [&str; 2] = [TEXT, JSON];

/// The class of an inquiry the classifier could not score: it is routed as `unsure`.
pub const UNSCORED: &str = "unscored";

/// The router's IRI.
pub const ROUTE: &str = "urn:iki:inquiry:route";
/// The sender-key door's IRI.
pub const SENDER_KEY: &str = "urn:iki:inquiry:sender-key";
/// The airlock's apply door's IRI.
pub const APPLY: &str = "urn:iki:inquiry:apply";
/// The block set's template.
pub const BLOCK: &str = "urn:iki:inquiry:block:key:{key}";
/// The day counters' template.
pub const COUNT: &str = "urn:iki:inquiry:count:date:{date}";
/// The day confusion counters' template.
pub const CONFUSION: &str = "urn:iki:inquiry:count:kind:confusion:date:{date}";

/// The capability the router requires.
pub const CAP_ROUTE: &str = "urn:cap:inquiry:route";
/// The capability the sender-key door requires.
pub const CAP_SENDER_KEY: &str = "urn:cap:inquiry:sender-key";
/// The capability the block door's check (`Exists`) requires.
pub const CAP_BLOCK_CHECK: &str = "urn:cap:inquiry:block:check";
/// The capability reading a block's record requires.
pub const CAP_BLOCK_READ: &str = "urn:cap:inquiry:block:read";
/// The capability lifting a block requires.
pub const CAP_BLOCK_LIFT: &str = "urn:cap:inquiry:block:lift";
/// The capability the airlock's apply door requires.
pub const CAP_APPLY: &str = "urn:cap:inquiry:apply";
/// The capability reading the counters requires.
pub const CAP_COUNT: &str = "urn:cap:inquiry:count:read";

/// What an intent may ask of apply: the words the intent shapes admit as `inq:intent`.
pub const INTENTS: [&str; 7] = [
    "train-ham",
    "train-spam",
    "untrain",
    "block",
    "unblock",
    "block-domain",
    "unblock-domain",
];

/// The longest block, in days.
pub const MAX_BLOCK_DAYS: u64 = 3650;

const DAY_MS: u64 = 86_400_000;

/// The edge half's space: the router, the sender-key door, the block set, apply and the
/// counters. Built from a host's [`Edge`], so the HOST names it.
pub fn edge_space(edge: &Arc<Edge>) -> EndpointSpace {
    let template = |t: &str| UriTemplate::parse(t).expect("a valid template");
    EndpointSpace::new()
        .bind(Exact::new(ROUTE), Route(edge.clone()))
        .bind(Exact::new(SENDER_KEY), SenderKeyDoor(edge.clone()))
        .bind(Exact::new(APPLY), Apply(edge.clone()))
        .bind(template(BLOCK), BlockDoor(edge.clone()))
        .bind(template(CONFUSION), CountDoor(edge.clone(), true))
        .bind(template(COUNT), CountDoor(edge.clone(), false))
}

// --- arguments ----------------------------------------------------------------------------

fn invalid(name: &str, detail: impl Into<String>) -> Error {
    Error::InvalidArgument {
        name: name.into(),
        detail: detail.into(),
    }
}

fn optional_str<'a>(inv: &'a Invocation<'_>, name: &str) -> Result<Option<&'a str>> {
    match inv.request.args.get(name) {
        None => Ok(None),
        Some(_) => inv.inline_str(name).map(Some),
    }
}

/// An optional field: absent when missing, empty or all whitespace (a form's blank field).
fn field<'a>(inv: &'a Invocation<'_>, name: &str) -> Result<Option<&'a str>> {
    let value = optional_str(inv, name)?.filter(|v| !v.trim().is_empty());
    if let Some(v) = value {
        if v.chars().count() > MAX_FIELD_CHARS {
            return Err(invalid(
                name,
                format!("longer than {MAX_FIELD_CHARS} characters: refused, not truncated"),
            ));
        }
    }
    Ok(value)
}

fn face(inv: &Invocation<'_>) -> Result<&'static str> {
    match optional_str(inv, "as")? {
        None => Ok(TEXT),
        Some(asked) => FACES.iter().copied().find(|f| *f == asked).ok_or_else(|| {
            invalid(
                "as",
                format!("`{asked}` is not one of {}", FACES.join(", ")),
            )
        }),
    }
}

fn now(inv: &Invocation<'_>) -> Result<Time> {
    inv.now()
        .ok_or_else(|| Error::Unavailable("no clock: the host gives the kernel one".into()))
}

fn binding<'a>(inv: &'a Invocation<'_>, name: &str) -> Result<&'a str> {
    inv.bindings
        .get(name)
        .ok_or_else(|| Error::MissingArgument(name.into()))
}

fn string_spec(name: &str, summary: &str) -> ArgSpec {
    ArgSpec::new(name)
        .summary(summary)
        .class(format!("{XSD}string"))
}

fn as_spec() -> ArgSpec {
    string_spec("as", "the representation to answer with")
        .one_of(FACES)
        .default_value(TEXT)
}

fn answer(face: &str, text: String, value: Value) -> Representation {
    if face == JSON {
        Representation::new(ReprType::new(JSON), value.to_string().into_bytes())
    } else {
        Representation::new(
            ReprType::new(TEXT).with_param("charset", "utf-8"),
            text.into_bytes(),
        )
    }
}

fn iri(text: String) -> Result<Iri> {
    Iri::parse(&text).map_err(|e| Error::Endpoint(format!("<{text}>: {e}")))
}

fn inline(request: Request, name: &str, value: &str) -> Request {
    request.with_arg(name, ArgRef::Inline(value.as_bytes().to_vec()))
}

/// The fields the classifier scores and is taught: the same for a score and for the training
/// that follows it, so a correction is the same message, moved.
struct Message<'a> {
    text: &'a str,
    email: Option<&'a str>,
    name: Option<&'a str>,
    organization: Option<&'a str>,
}

impl<'a> Message<'a> {
    fn of_arrival(arrival: &'a Arrival) -> Self {
        Message {
            text: &arrival.text,
            email: Some(&arrival.email),
            name: arrival.name.as_deref(),
            organization: arrival.organization.as_deref(),
        }
    }

    fn of_copy(copy: &'a layer::Inquiry) -> Self {
        Message {
            text: &copy.text,
            email: copy.reply_to.as_deref().map(str::trim),
            name: copy.name.as_deref(),
            organization: copy.organization.as_deref(),
        }
    }

    /// The fields as the classifier takes them (its field for the organization is
    /// `organisation`).
    fn onto(&self, mut request: Request) -> Request {
        request = inline(request, "content", self.text);
        for (name, value) in [
            ("email", self.email),
            ("name", self.name),
            ("organisation", self.organization),
        ] {
            if let Some(value) = value {
                request = inline(request, name, value);
            }
        }
        request
    }
}

fn is_site(site: &str) -> bool {
    (1..=MAX_SITE_CHARS).contains(&site.len())
        && site.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && site
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

// --- route --------------------------------------------------------------------------------

/// `urn:iki:inquiry:route`: score an inquiry, pick its layer, drop it there once.
struct Route(Arc<Edge>);

impl Route {
    async fn score(&self, inv: &Invocation<'_>, arrival: &Arrival) -> Result<(String, f64)> {
        let target = iri(format!("urn:bayes:{}:score", self.0.config.model))?;
        let request = inline(
            inline(
                Message::of_arrival(arrival).onto(Request::new(Verb::Source, target)),
                "clues",
                "0",
            ),
            "as",
            JSON,
        );
        let rep = inv.issue(request).await?;
        let value: Value = serde_json::from_slice(&rep.bytes)
            .map_err(|e| Error::Endpoint(format!("the classifier's answer is not JSON: {e}")))?;
        let class = value.get("class").and_then(Value::as_str);
        let score = value.get("score").and_then(Value::as_f64);
        match (class, score) {
            (Some(class), Some(score)) if (0.0..=1.0).contains(&score) => {
                Ok((class.to_string(), score))
            }
            _ => Err(Error::Endpoint(
                "the classifier's answer has no class and score".into(),
            )),
        }
    }
}

#[async_trait]
impl Endpoint for Route {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        let edge = &self.0;
        let face = face(inv)?;
        let now = now(inv)?;
        let site = inv.inline_str("site")?;
        if !is_site(site) {
            return Err(invalid(
                "site",
                format!(
                    "a site is 1 to {MAX_SITE_CHARS} lowercase letters, digits, dots and \
                     hyphens, starting with a letter or digit"
                ),
            ));
        }
        let email = inv.inline_str("email")?.trim();
        let text = inv.inline_str("content")?;
        let name = field(inv, "name")?;
        let organization = field(inv, "organization")?;
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(invalid(
                "content",
                format!(
                    "the message is longer than {MAX_TEXT_CHARS} characters: refused, not \
                     truncated"
                ),
            ));
        }
        let size =
            text.len() + email.len() + name.map_or(0, str::len) + organization.map_or(0, str::len);
        if size > MAX_MESSAGE_BYTES {
            return Err(invalid(
                "content",
                format!(
                    "the inquiry is {size} bytes, past the bound of {MAX_MESSAGE_BYTES}: \
                     refused, not truncated"
                ),
            ));
        }
        let sender_key = edge.keys.address(email).map_err(|e| invalid("email", e))?;
        let domain_key = edge
            .keys
            .domain_of(email)
            .map_err(|e| invalid("email", e))?;
        let date = time::date(now);
        if edge.blocks.holding(&sender_key, now).is_some()
            || edge.blocks.holding(&domain_key, now).is_some()
        {
            edge.counts
                .add(&date, Tally::Blocked { site })
                .map_err(Error::Endpoint)?;
            return Ok(answer(face, "blocked\n".into(), json!({ "blocked": true })));
        }
        let received = time::date_time(now);
        let mut inquiry = Arrival {
            id: edge.keys.inquiry_id(&[
                site,
                &received,
                email,
                name.unwrap_or(""),
                organization.unwrap_or(""),
                text,
            ]),
            site: site.to_string(),
            name: name.map(String::from),
            email: email.to_string(),
            organization: organization.map(String::from),
            text: text.to_string(),
            sender_key: sender_key.to_string(),
            received,
            class: UNSCORED.into(),
            class_score: None,
            reason: UNSCORED.into(),
        };
        // A classifier that cannot answer does not lose the inquiry: it goes where an unsure
        // one goes, marked unscored, for a person to judge.
        let class = match self.score(inv, &inquiry).await {
            Ok((class, score)) => {
                inquiry.class_score = Some(score);
                class
            }
            Err(_) => UNSCORED.to_string(),
        };
        let layer = edge.config.policy.layer_for(&class).ok_or_else(|| {
            Error::Endpoint(format!(
                "the classifier answered the class `{class}`, which the policy does not route"
            ))
        })?;
        inquiry.class = class.clone();
        if inquiry.class_score.is_some() {
            inquiry.reason = "score".into();
        }
        let turtle = inquiry.to_turtle(layer).map_err(Error::Endpoint)?;
        let out = Request::new(Verb::Sink, iri(format!("urn:layer:{layer}:tuples"))?)
            .with_arg("content", ArgRef::Inline(turtle));
        inv.issue(out).await?;
        edge.counts
            .add(&date, Tally::Routed { site, layer })
            .map_err(Error::Endpoint)?;
        let tuple = rdf::subject(layer, &inquiry.id);
        Ok(answer(
            face,
            format!("{class} {layer} {tuple}\n"),
            json!({
                "class": class,
                "id": inquiry.id,
                "layer": layer,
                "score": inquiry.class_score,
                "tuple": tuple,
            }),
        ))
    }

    fn name(&self) -> &str {
        "inquiry-route"
    }

    fn describe(&self) -> Description {
        Description::new("inquiry-route")
            .title("Route an inquiry from a contact form")
            .summary(
                "Scores an inquiry with the Bayes classifier, picks a layer from the policy \
                 (ham to the inquiry layer, unsure to holding, spam to spam) and drops ONE \
                 inq:Inquiry tuple into it. A sender whose address or domain is blocked is \
                 dropped and counted, never stored. Answers the class, the layer and the \
                 tuple's subject.",
            )
            .verb(Verb::Sink)
            .verb(Verb::Meta)
            .requires(CAP_ROUTE)
            .input(string_spec("content", "the message (piped)"))
            .input(string_spec("site", "the site whose form it came from"))
            .input(string_spec("email", "the sender's address"))
            .input(string_spec("name", "the sender's name").optional())
            .input(string_spec("organization", "the sender's organization").optional())
            .input(as_spec())
            .output(TEXT)
            .output(JSON)
    }
}

// --- sender key ---------------------------------------------------------------------------

/// `urn:iki:inquiry:sender-key`: the key of an address or a domain.
struct SenderKeyDoor(Arc<Edge>);

#[async_trait]
impl Endpoint for SenderKeyDoor {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        let face = face(inv)?;
        let (key, scope) = match (optional_str(inv, "address")?, optional_str(inv, "domain")?) {
            (Some(address), None) => (
                self.0
                    .keys
                    .address(address)
                    .map_err(|e| invalid("address", e))?,
                Scope::Address,
            ),
            (None, Some(domain)) => (
                self.0
                    .keys
                    .domain(domain)
                    .map_err(|e| invalid("domain", e))?,
                Scope::Domain,
            ),
            _ => return Err(invalid("address", "give exactly one of address and domain")),
        };
        // Not cacheable: the request holds the address, and a cache would keep it.
        Ok(answer(
            face,
            format!("{key}\n"),
            json!({ "key": key.as_str(), "scope": scope.as_str() }),
        ))
    }

    fn name(&self) -> &str {
        "inquiry-sender-key"
    }

    fn describe(&self) -> Description {
        Description::new("inquiry-sender-key")
            .title("The sender key of an address or a domain")
            .summary(
                "Keyed BLAKE3 over the address, trimmed and lowercased, or over \
                 `domain:<the domain, lowercased>`, under the host's key. Nothing \
                 provider-specific is folded.",
            )
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .requires(CAP_SENDER_KEY)
            .input(string_spec("address", "an address").optional())
            .input(string_spec("domain", "a domain").optional())
            .input(as_spec())
            .output(TEXT)
            .output(JSON)
    }
}

// --- the block set ------------------------------------------------------------------------

/// `urn:iki:inquiry:block:key:{key}`: Exists is the door's check, Source the record, Delete
/// lifts it.
struct BlockDoor(Arc<Edge>);

fn block_json(block: &Block) -> Value {
    json!({
        "created": time::date_time(block.created),
        "expires": time::date_time(block.expires),
        "key": block.key.as_str(),
        "label": block.label,
        "scope": block.scope.as_str(),
    })
}

fn block_text(block: &Block) -> String {
    let mut out = format!("key {}\nscope {}\n", block.key, block.scope.as_str());
    if let Some(label) = &block.label {
        out.push_str(&format!("label {label}\n"));
    }
    out.push_str(&format!(
        "created {}\nexpires {}\n",
        time::date_time(block.created),
        time::date_time(block.expires)
    ));
    out
}

#[async_trait]
impl Endpoint for BlockDoor {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        let key = Key::parse(binding(inv, "key")?).map_err(|e| invalid("key", e))?;
        let now = now(inv)?;
        let blocks = &self.0.blocks;
        let none = || Error::NotFound(format!("no block holds on {key}"));
        // Never cacheable: a block expires, and is lifted, while a cached answer would not.
        match inv.request.verb {
            Verb::Exists => {
                blocks.holding(&key, now).ok_or_else(none)?;
                Ok(Representation::new(
                    ReprType::new(TEXT).with_param("charset", "utf-8"),
                    b"blocked\n".to_vec(),
                ))
            }
            Verb::Source => {
                let face = face(inv)?;
                let block = blocks.holding(&key, now).ok_or_else(none)?;
                Ok(answer(face, block_text(&block), block_json(&block)))
            }
            Verb::Delete => {
                let face = face(inv)?;
                let lifted = blocks.lift(&key, now).map_err(Error::Endpoint)?;
                Ok(lift_answer(face, &key, lifted.is_some()))
            }
            verb => Err(Error::Endpoint(format!(
                "{verb:?} is not a block door's verb"
            ))),
        }
    }

    fn name(&self) -> &str {
        "inquiry-block"
    }

    fn describe(&self) -> Description {
        let key = ArgSpec::new("key")
            .summary("the sender key: 64 lowercase hex digits")
            .class(format!("{XSD}string"))
            .binding();
        Description::new("inquiry-block")
            .title("A block on a sender key")
            .summary(
                "One record per block: the key, its scope (address or domain), an optional \
                 label, when it was written and when it expires; never an address. Exists is \
                 the door's check and answers NotFound once the block has expired; Delete \
                 lifts it.",
            )
            .verb(Verb::Meta)
            .action(
                ActionSpec::new(Verb::Exists)
                    .summary("whether a block holds on the key now (NotFound if not)")
                    .input(key.clone())
                    .output(TEXT)
                    .requires(CAP_BLOCK_CHECK),
            )
            .action(
                ActionSpec::new(Verb::Source)
                    .summary("the block's record")
                    .input(key.clone())
                    .input(as_spec())
                    .output(TEXT)
                    .output(JSON)
                    .requires(CAP_BLOCK_READ),
            )
            .action(
                ActionSpec::new(Verb::Delete)
                    .summary("lift the block (idempotent)")
                    .input(key)
                    .input(as_spec())
                    .output(TEXT)
                    .output(JSON)
                    .requires(CAP_BLOCK_LIFT),
            )
    }
}

fn lift_parts(key: &Key, lifted: bool) -> Applied {
    let text = if lifted {
        format!("lifted {key}\n")
    } else {
        format!("no block on {key}\n")
    };
    (text, json!({ "key": key.as_str(), "lifted": lifted }))
}

fn lift_answer(face: &str, key: &Key, lifted: bool) -> Representation {
    let (text, value) = lift_parts(key, lifted);
    answer(face, text, value)
}

// --- apply --------------------------------------------------------------------------------

/// `urn:iki:inquiry:apply`: the airlock's handler. It takes a person's intents from the
/// intents layer, one at a time in feed order: each is applied, then taken.
struct Apply(Arc<Edge>);

/// What one intent did: its line, its JSON.
type Applied = (String, Value);

impl Apply {
    /// The pending layer copy of inquiry `id` in one of the policy's layers, if there is one.
    async fn layer_copy(&self, inv: &Invocation<'_>, id: &str) -> Result<Option<layer::Inquiry>> {
        for name in self.0.config.policy.layers() {
            if let Some(copy) = layer::find(inv, name, id).await? {
                return Ok(Some(copy));
            }
        }
        Ok(None)
    }

    async fn train(
        &self,
        inv: &Invocation<'_>,
        intent: &Intent,
        label: &str,
        date: &str,
    ) -> Result<Applied> {
        let copy = self.layer_copy(inv, &intent.inquiry).await?;
        let message = training_message(intent, copy.as_ref())?;
        let model = &self.0.config.model;
        let request = Request::new(Verb::Sink, iri(format!("urn:bayes:{model}:train"))?);
        let request = inline(inline(message.onto(request), "label", label), "as", JSON);
        let rep = inv.issue(request).await?;
        let value: Value = serde_json::from_slice(&rep.bytes).unwrap_or(Value::Null);
        let change = value
            .get("change")
            .and_then(Value::as_str)
            .unwrap_or("unchanged")
            .to_string();
        // The confusion counts each label a person's judgment changed: a repeat is not counted
        // again. The class is the edge's, which every copy and train intent carries.
        if change != "unchanged" {
            let class = copy
                .as_ref()
                .and_then(|c| c.class.as_deref())
                .or(intent.class.as_deref())
                .unwrap_or("unknown");
            self.0
                .counts
                .add(date, Tally::Labeled { class, label })
                .map_err(Error::Endpoint)?;
        }
        let id = &intent.inquiry;
        Ok((
            format!("trained {label} ({change}) {id}\n"),
            json!({ "change": change, "inquiry": id, "intent": intent.subject, "label": label }),
        ))
    }

    async fn untrain(&self, inv: &Invocation<'_>, intent: &Intent) -> Result<Applied> {
        let copy = self.layer_copy(inv, &intent.inquiry).await?;
        let message = training_message(intent, copy.as_ref())?;
        let model = &self.0.config.model;
        let request = Request::new(Verb::Delete, iri(format!("urn:bayes:{model}:untrain"))?);
        inv.issue(message.onto(request)).await?;
        let id = &intent.inquiry;
        Ok((
            format!("untrained {id}\n"),
            json!({ "inquiry": id, "intent": intent.subject, "untrained": true }),
        ))
    }

    async fn block(
        &self,
        inv: &Invocation<'_>,
        intent: &Intent,
        scope: Scope,
        blocking: bool,
        now: Time,
    ) -> Result<Applied> {
        let edge = &self.0;
        let id = &intent.inquiry;
        let key = match (scope, &intent.sender) {
            (Scope::Address, Some(sender)) => Key::parse(sender)
                .map_err(|e| Error::NotFound(format!("the intent's sender key: {e}")))?,
            _ => {
                let copy = self.layer_copy(inv, id).await?.ok_or_else(|| {
                    Error::NotFound(format!(
                        "no sender key for inquiry {id}: no pending copy in the policy's layers"
                    ))
                })?;
                match scope {
                    Scope::Address => copy.sender.as_deref().map(Key::parse),
                    Scope::Domain => copy
                        .reply_to
                        .as_deref()
                        .map(|address| edge.keys.domain_of(address)),
                }
                .ok_or_else(|| Error::NotFound(format!("inquiry {id} has no sender to key")))?
                .map_err(|e| Error::NotFound(format!("inquiry {id}'s sender: {e}")))?
            }
        };
        let subject = &intent.subject;
        if let Some(note) = &intent.note {
            check_label(note).map_err(|e| invalid("note", e))?;
        }
        if !blocking {
            let lifted = edge.blocks.lift(&key, now).map_err(Error::Endpoint)?;
            let (text, mut value) = lift_parts(&key, lifted.is_some());
            value["intent"] = json!(subject);
            return Ok((text, value));
        }
        let block = Block {
            key: key.clone(),
            scope,
            label: intent.note.clone(),
            created: now,
            expires: now.plus_millis(edge.config.block_days * DAY_MS),
        };
        edge.blocks
            .put(block.clone(), now)
            .map_err(Error::Endpoint)?;
        Ok((
            format!(
                "blocked {} {key} until {}\n",
                scope.as_str(),
                time::date_time(block.expires)
            ),
            json!({ "block": block_json(&block), "intent": subject }),
        ))
    }

    async fn apply(
        &self,
        inv: &Invocation<'_>,
        intent: &Intent,
        now: Time,
        date: &str,
    ) -> Result<Applied> {
        match intent.intent.as_str() {
            "train-ham" => self.train(inv, intent, "ham", date).await,
            "train-spam" => self.train(inv, intent, "spam", date).await,
            "untrain" => self.untrain(inv, intent).await,
            "block" => self.block(inv, intent, Scope::Address, true, now).await,
            "unblock" => self.block(inv, intent, Scope::Address, false, now).await,
            "block-domain" => self.block(inv, intent, Scope::Domain, true, now).await,
            "unblock-domain" => self.block(inv, intent, Scope::Domain, false, now).await,
            other => Err(Error::NotFound(format!(
                "`{other}` is not one of {}",
                INTENTS.join(", ")
            ))),
        }
    }
}

/// What to train on: the inquiry's layer copy, with every field it was scored with; or, when
/// no copy is pending any more (a confirmed spam was taken), the text the intent carries.
fn training_message<'a>(
    intent: &'a Intent,
    copy: Option<&'a layer::Inquiry>,
) -> Result<Message<'a>> {
    match (copy, &intent.text) {
        (Some(copy), _) => Ok(Message::of_copy(copy)),
        (None, Some(text)) => Ok(Message {
            text,
            email: None,
            name: None,
            organization: None,
        }),
        (None, None) => Err(Error::NotFound(format!(
            "nothing to train on for inquiry {}: no pending copy, and the intent carries no text",
            intent.inquiry
        ))),
    }
}

#[async_trait]
impl Endpoint for Apply {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        let edge = &self.0;
        let face = face(inv)?;
        let now = now(inv)?;
        let date = time::date(now);
        let layer = &edge.config.intents;
        let mut pending = layer::intents(inv, layer).await?;
        if let Some(subject) = optional_str(inv, "intent")? {
            pending.retain(|i| i.subject == subject);
            if pending.is_empty() {
                return Err(Error::NotFound(format!(
                    "no intent <{subject}> is pending in layer `{layer}`"
                )));
            }
        }
        let (mut text, mut values) = (String::new(), Vec::new());
        for intent in &pending {
            // An intent that can never apply (nothing to key, never trained, a label a block
            // cannot carry) is taken and reported, so it does not stand in front of the rest. Any other failure stops
            // here and leaves it pending for the next run: what was applied before it stays
            // applied and taken, and applying an intent again changes nothing.
            let (line, value) = match self.apply(inv, intent, now, &date).await {
                Ok(applied) => applied,
                Err(Error::NotFound(why) | Error::InvalidArgument { detail: why, .. }) => (
                    format!("skipped {} {}: {why}\n", intent.intent, intent.inquiry),
                    json!({
                        "inquiry": intent.inquiry,
                        "intent": intent.subject,
                        "skipped": why,
                    }),
                ),
                Err(e) => return Err(e),
            };
            layer::take(inv, layer, &intent.subject).await?;
            text.push_str(&line);
            values.push(value);
        }
        if pending.is_empty() {
            text.push_str("no intents pending\n");
        }
        Ok(answer(face, text, json!({ "applied": values })))
    }

    fn name(&self) -> &str {
        "inquiry-apply"
    }

    fn describe(&self) -> Description {
        Description::new("inquiry-apply")
            .title("Apply the intents people wrote about inquiries")
            .summary(
                "The airlock's handler. Takes each pending intent from the intents layer, in \
                 feed order, applies it, then takes it. train-ham and train-spam train the \
                 classifier from the inquiry's layer copy (or the intent's text, when no copy \
                 is pending) and count the edge's class against the label; untrain makes it \
                 forget; block and block-domain write a block on the sender's key or its \
                 domain's key; unblock and unblock-domain lift one.",
            )
            .verb(Verb::Sink)
            .verb(Verb::Meta)
            .requires(CAP_APPLY)
            .input(
                ArgSpec::new("intent")
                    .summary(
                        "one intent's tuple, urn:layer:{layer}:intent:name:…; every one if absent",
                    )
                    .class(format!("{XSD}anyURI"))
                    .optional(),
            )
            .input(as_spec())
            .output(TEXT)
            .output(JSON)
    }
}

// --- the counters -------------------------------------------------------------------------

/// `urn:iki:inquiry:count:date:{date}` and `…:kind:confusion:date:{date}`.
struct CountDoor(Arc<Edge>, bool);

#[async_trait]
impl Endpoint for CountDoor {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        let face = face(inv)?;
        let date = binding(inv, "date")?;
        if !time::is_date(date) {
            return Err(invalid(
                "date",
                format!("`{date}` is not a date, YYYY-MM-DD"),
            ));
        }
        let day = self.0.counts.day(date).map_err(Error::Endpoint)?;
        // Never cacheable: today's counts move with every inquiry.
        Ok(if self.1 {
            answer(face, day.confusion_text(date), day.confusion_json(date))
        } else {
            answer(face, day.sites_text(date), day.sites_json(date))
        })
    }

    fn name(&self) -> &str {
        if self.1 {
            "inquiry-confusion"
        } else {
            "inquiry-count"
        }
    }

    fn describe(&self) -> Description {
        let (title, summary) = if self.1 {
            (
                "A day's confusion counts",
                "For each score class (ham, unsure, spam, unscored), how many inquiries a \
                 person labeled spam and how many ham, that day. Aggregates only.",
            )
        } else {
            (
                "A day's routing counts",
                "Per site, how many inquiries went to each layer and how many a block \
                 dropped, that day (UTC). Aggregates only: nothing tracks an individual.",
            )
        };
        Description::new(self.name())
            .title(title)
            .summary(summary)
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .requires(CAP_COUNT)
            .input(
                ArgSpec::new("date")
                    .summary("the day, YYYY-MM-DD (UTC)")
                    .class(format!("{XSD}date"))
                    .binding(),
            )
            .input(as_spec())
            .output(TEXT)
            .output(JSON)
    }
}
