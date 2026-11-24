//! Talking to the layer doors. Every read and write is a sub-request under the caller's own
//! capability: neither the layers client nor the holding stage holds authority of its own, so
//! a person or a worker does through them exactly what their layer grants let them do, and
//! nothing more.
//!
//! Every error built here names layers, subjects and ids, never an inquiry's text.

use ikigai_core::{ArgRef, Error, Invocation, Iri, Request, Result, Verb};
use serde_json::{json, Map, Value};

use crate::example;
use crate::funnel::Embedding;
use crate::vocab::{inq, XSD, XSD_DATE_TIME};

/// The longest page a layer answers (`ikigai-layers`' `MAX_LIMIT`).
const PAGE: u64 = 1000;

const JSON: &str = "application/json";
const JSON_LD: &str = "application/ld+json";

/// `text` as an IRI; one this module built that does not parse is its own defect.
pub(crate) fn iri(text: &str) -> Result<Iri> {
    Iri::parse(text).map_err(|e| Error::Endpoint(format!("`{text}` is not an IRI: {e}")))
}

pub(crate) fn request(verb: Verb, target: Iri, args: &[(&str, &str)]) -> Request {
    let mut request = Request::new(verb, target);
    for (name, value) in args {
        request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    request
}

pub(crate) fn json_answer(bytes: &[u8], what: &str) -> Result<Value> {
    serde_json::from_slice(bytes)
        .map_err(|e| Error::Endpoint(format!("{what}: the answer is not JSON: {e}")))
}

/// `urn:layer:{layer}:tuples`.
fn tuples(layer: &str) -> Result<Iri> {
    iri(&format!("urn:layer:{layer}:tuples"))
}

/// The subject inquiry `id` has in `layer`: `urn:layer:{layer}:inquiry:id:{id}`.
pub fn inquiry_subject(layer: &str, id: &str) -> String {
    format!("urn:layer:{layer}:inquiry:id:{id}")
}

/// A node's value for the term `local` as one string. A layer's JSON-LD face compacts with the
/// layer's context, so the key is the term (`text`), its compact IRI (`inq:text`) or the full
/// IRI, and the value a string, a value object or an array of either.
pub(crate) fn value(node: &Value, local: &str) -> Option<String> {
    let found = [local.to_string(), format!("inq:{local}"), inq(local)]
        .iter()
        .find_map(|key| node.get(key.as_str()))?;
    let first = match found {
        Value::Array(values) => values.first()?,
        other => other,
    };
    match first {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Object(o) => o.get("@value").and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }),
        _ => None,
    }
}

/// Whether a node is typed `local` (`Inquiry`, `SpamExample`).
pub(crate) fn has_type(node: &Value, local: &str) -> bool {
    let full = inq(local);
    let compact = format!("inq:{local}");
    let is = |t: &Value| t == local || t == full.as_str() || t == compact.as_str();
    match node.get("@type") {
        Some(Value::String(_)) => is(&node["@type"]),
        Some(Value::Array(types)) => types.iter().any(is),
        _ => false,
    }
}

/// The nodes of a layer's JSON-LD answer: always one top-level `@graph`.
pub(crate) fn graph(doc: &Value) -> Vec<Value> {
    doc.get("@graph")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// One inquiry as the layers client reads it: a pending tuple in an inquiry, spam or holding
/// layer, with its subject and its place in the feed.
///
/// `text`, `reply_to`, `name` and `organization` are a stranger's words. They are carried,
/// compared and escaped, never interpreted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inquiry {
    /// The tuple's IRI, `urn:layer:{layer}:inquiry:id:…`.
    pub subject: String,
    /// The inquiry's id, the same in every layer a move takes it to.
    pub id: String,
    /// The site whose form it came from.
    pub site: String,
    /// What the visitor wrote.
    pub text: String,
    /// The address the visitor gave for a reply, as typed.
    pub reply_to: Option<String>,
    /// The name the visitor gave.
    pub name: Option<String>,
    /// The organization the visitor gave.
    pub organization: Option<String>,
    /// The edge's keyed hash of the sender.
    pub sender: Option<String>,
    /// What sent it to this layer.
    pub reason: Option<String>,
    /// The edge's class: `ham`, `unsure`, `spam` or `unscored`.
    pub class: Option<String>,
    /// The classifier's score behind the class, as the layer gives it (an `xsd:double`).
    pub class_score: Option<String>,
    /// When the edge received it (`xsd:dateTime`), or when the layer stamped this copy
    /// when the edge did not say.
    pub received: String,
    /// This copy's place in its layer's feed.
    pub seq: u64,
}

/// One inquiry as the holding stage takes it from a holding tuple. `text`, `reply_to`, `name`
/// and `organization` are a stranger's words: carried and compared, never interpreted, and
/// never put in an error or an answer.
#[derive(Clone, PartialEq, Eq)]
pub struct Held {
    /// The inquiry's id, the same in every layer it goes to.
    pub id: String,
    /// The site whose form it came from.
    pub site: String,
    /// What the visitor wrote.
    pub text: String,
    /// The address the visitor gave for a reply, as typed.
    pub reply_to: Option<String>,
    /// The name the visitor gave.
    pub name: Option<String>,
    /// The organization the visitor gave.
    pub organization: Option<String>,
    /// The edge's keyed hash of the sender.
    pub sender: Option<String>,
    /// When the edge received it (`xsd:dateTime`).
    pub received: Option<String>,
    /// The edge's class and the classifier's score, carried on so that apply counts and
    /// trains what the edge scored.
    pub class: Option<String>,
    /// See `class`.
    pub class_score: Option<String>,
}

// Never print the text: a Debug that did would put a stranger's words in a log.
impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Held")
            .field("id", &self.id)
            .field("site", &self.site)
            .field("text", &format_args!("[{} bytes]", self.text.len()))
            .finish_non_exhaustive()
    }
}

impl Held {
    /// The inquiry in a tuple graph (a take's answer, or a receipt): the one `inq:Inquiry`
    /// node, or `None` when there is none that carries an id, a site and a text.
    pub fn from_graph(nodes: &[Value]) -> Option<Held> {
        let node = nodes.iter().find(|n| has_type(n, "Inquiry"))?;
        Some(Held {
            id: value(node, "inquiry")?,
            site: value(node, "site")?,
            text: value(node, "text")?,
            reply_to: value(node, "replyTo"),
            name: value(node, "name"),
            organization: value(node, "organization"),
            sender: value(node, "sender"),
            received: value(node, "received"),
            class: value(node, "class"),
            class_score: value(node, "classScore"),
        })
    }

    /// This inquiry as a tuple of `layer`, with `extra` properties (full IRI → JSON-LD value),
    /// written in full IRIs so it reads the same under any context.
    pub(crate) fn tuple(&self, layer: &str, extra: Map<String, Value>) -> Value {
        let mut node = Map::new();
        node.insert("@id".into(), json!(inquiry_subject(layer, &self.id)));
        node.insert("@type".into(), json!(inq("Inquiry")));
        node.insert(inq("inquiry"), json!(self.id));
        node.insert(inq("site"), json!(self.site));
        node.insert(inq("text"), json!(self.text));
        if let Some(reply_to) = &self.reply_to {
            node.insert(inq("replyTo"), json!(reply_to));
        }
        if let Some(sender) = &self.sender {
            node.insert(inq("sender"), json!(sender));
        }
        if let Some(received) = &self.received {
            node.insert(
                inq("received"),
                json!({ "@value": received, "@type": format!("{XSD}dateTime") }),
            );
        }
        edge_fields(
            &mut node,
            self.name.as_deref(),
            self.organization.as_deref(),
            self.class.as_deref(),
            self.class_score.as_deref(),
        );
        node.extend(extra);
        Value::Object(node)
    }
}

/// The fields the edge writes that every later copy keeps: the sender's name and organization,
/// and the edge's class and score.
fn edge_fields(
    node: &mut Map<String, Value>,
    name: Option<&str>,
    organization: Option<&str>,
    class: Option<&str>,
    class_score: Option<&str>,
) {
    for (term, value) in [
        ("name", name),
        ("organization", organization),
        ("class", class),
    ] {
        if let Some(value) = value {
            node.insert(inq(term), json!(value));
        }
    }
    if let Some(score) = class_score {
        node.insert(
            inq("classScore"),
            json!({ "@value": score, "@type": format!("{XSD}double") }),
        );
    }
}

/// One pending tuple, as `rd`'s JSON list gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// Its place in the layer's feed.
    pub seq: u64,
    /// When it was dropped, as `xsd:dateTime`.
    pub created: String,
    /// Its name.
    pub subject: String,
}

/// Every pending tuple of `layer`, in feed order, paged: its listing, without the tuples.
pub async fn listed(inv: &Invocation<'_>, layer: &str) -> Result<Vec<Listed>> {
    let mut all = Vec::new();
    let mut after = 0;
    loop {
        let (after_s, limit) = (after.to_string(), PAGE.to_string());
        let answer = inv
            .issue(request(
                Verb::Source,
                tuples(layer)?,
                &[("as", JSON), ("after", &after_s), ("limit", &limit)],
            ))
            .await?;
        let doc = json_answer(&answer.bytes, layer)?;
        let rows = doc["tuples"].as_array().cloned().unwrap_or_default();
        let n = rows.len() as u64;
        for row in rows {
            let (Some(seq), Some(created), Some(subject)) = (
                row["seq"].as_u64(),
                row["created"].as_str(),
                row["subject"].as_str(),
            ) else {
                return Err(Error::Endpoint(format!(
                    "layer `{layer}` listed a tuple without its seq, created and subject"
                )));
            };
            after = after.max(seq);
            all.push(Listed {
                seq,
                created: created.into(),
                subject: subject.into(),
            });
        }
        if n < PAGE {
            return Ok(all);
        }
    }
}

/// One take, as `rd state=taken`'s JSON list gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Taken {
    /// The subject of the tuple taken.
    pub subject: String,
    /// Who took it.
    pub taker: String,
    /// The receipt's IRI.
    pub receipt: String,
}

/// Every take from `layer`, in take order, paged.
pub async fn receipts(inv: &Invocation<'_>, layer: &str) -> Result<Vec<Taken>> {
    let mut all = Vec::new();
    let mut after = 0;
    loop {
        let (after_s, limit) = (after.to_string(), PAGE.to_string());
        let answer = inv
            .issue(request(
                Verb::Source,
                tuples(layer)?,
                &[
                    ("state", "taken"),
                    ("as", JSON),
                    ("after", &after_s),
                    ("limit", &limit),
                ],
            ))
            .await?;
        let doc = json_answer(&answer.bytes, layer)?;
        let rows = doc["receipts"].as_array().cloned().unwrap_or_default();
        let n = rows.len() as u64;
        for row in rows {
            let (Some(seq), Some(subject), Some(taker), Some(receipt)) = (
                row["seq"].as_u64(),
                row["subject"].as_str(),
                row["taker"].as_str(),
                row["receipt"].as_str(),
            ) else {
                return Err(Error::Endpoint(format!(
                    "layer `{layer}` listed a receipt without its seq, subject, taker and IRI"
                )));
            };
            after = after.max(seq);
            all.push(Taken {
                subject: subject.into(),
                taker: taker.into(),
                receipt: receipt.into(),
            });
        }
        if n < PAGE {
            return Ok(all);
        }
    }
}

/// A receipt's graph: the take, and the tuple as it was taken.
pub async fn receipt(inv: &Invocation<'_>, receipt: &str) -> Result<Vec<Value>> {
    let answer = inv
        .issue(request(Verb::Source, iri(receipt)?, &[("as", JSON_LD)]))
        .await?;
    Ok(graph(&json_answer(&answer.bytes, receipt)?))
}

/// Take `subject` from `layer`, atomically: its graph as it was taken. A second taker is
/// answered the layer's `Conflict` naming the first.
pub async fn take(inv: &Invocation<'_>, layer: &str, subject: &str) -> Result<Vec<Value>> {
    let answer = inv
        .issue(request(
            Verb::Delete,
            tuples(layer)?,
            &[("tuple", subject), ("as", JSON_LD)],
        ))
        .await?;
    Ok(graph(&json_answer(&answer.bytes, layer)?))
}

/// OUT: drop `tuple` into `layer`.
pub async fn out(inv: &Invocation<'_>, layer: &str, tuple: &Value) -> Result<()> {
    inv.issue(request(
        Verb::Sink,
        tuples(layer)?,
        &[("content", &tuple.to_string())],
    ))
    .await
    .map(drop)
}

/// Whether `layer` has ever held `subject`: pending, or taken since.
pub async fn holds(inv: &Invocation<'_>, layer: &str, subject: &str) -> Result<bool> {
    for state in ["pending", "taken"] {
        match inv
            .issue(request(
                Verb::Source,
                tuples(layer)?,
                &[("tuple", subject), ("state", state)],
            ))
            .await
        {
            Ok(_) => return Ok(true),
            Err(Error::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(false)
}

/// Every entry of the channel `layer`, as JSON-LD nodes, paged.
pub async fn entries(inv: &Invocation<'_>, layer: &str) -> Result<Vec<Value>> {
    let target = iri(&format!("urn:layer:{layer}:entries"))?;
    let mut all = Vec::new();
    let mut after = 0;
    loop {
        let (after_s, limit) = (after.to_string(), PAGE.to_string());
        let answer = inv
            .issue(request(
                Verb::Source,
                target.clone(),
                &[("as", JSON_LD), ("after", &after_s), ("limit", &limit)],
            ))
            .await?;
        let nodes = graph(&json_answer(&answer.bytes, layer)?);
        let mut entries = 0;
        for node in nodes {
            if let Some(seq) = value(&node, "seq").and_then(|s| s.parse::<u64>().ok()) {
                after = after.max(seq);
                entries += 1;
            }
            all.push(node);
        }
        if entries < PAGE {
            return Ok(all);
        }
    }
}

fn inquiry(node: &Value) -> Option<Inquiry> {
    if !has_type(node, "Inquiry") {
        return None;
    }
    let created = value(node, "created");
    Some(Inquiry {
        subject: node.get("@id")?.as_str()?.to_string(),
        id: value(node, "inquiry")?,
        site: value(node, "site")?,
        text: value(node, "text")?,
        reply_to: value(node, "replyTo"),
        name: value(node, "name"),
        organization: value(node, "organization"),
        sender: value(node, "sender"),
        reason: value(node, "reason"),
        class: value(node, "class"),
        class_score: value(node, "classScore"),
        received: value(node, "received").or(created).unwrap_or_default(),
        seq: value(node, "seq")?.parse().ok()?,
    })
}

/// Every tuple pending in the tuplespace `layer`, as JSON-LD nodes, paged. The layer host
/// answers a default page of 100 when no `limit=` is given, so one unpaged read silently
/// misses every tuple past the 100th.
async fn pending_nodes(inv: &Invocation<'_>, layer: &str) -> Result<Vec<Value>> {
    let mut all = Vec::new();
    let mut after = 0;
    loop {
        let (after_s, limit) = (after.to_string(), PAGE.to_string());
        let answer = inv
            .issue(request(
                Verb::Source,
                tuples(layer)?,
                &[("as", JSON_LD), ("after", &after_s), ("limit", &limit)],
            ))
            .await?;
        let nodes = graph(&json_answer(&answer.bytes, layer)?);
        let mut listed = 0;
        for node in nodes {
            if let Some(seq) = value(&node, "seq").and_then(|s| s.parse::<u64>().ok()) {
                after = after.max(seq);
                listed += 1;
            }
            all.push(node);
        }
        if listed < PAGE {
            return Ok(all);
        }
    }
}

/// The inquiries pending in `layer`, in feed order, read whole (every page).
pub async fn pending(inv: &Invocation<'_>, layer: &str) -> Result<Vec<Inquiry>> {
    Ok(pending_nodes(inv, layer)
        .await?
        .iter()
        .filter_map(inquiry)
        .collect())
}

/// The pending copy of inquiry `id` in `layer`, if there is one.
pub async fn find(inv: &Invocation<'_>, layer: &str, id: &str) -> Result<Option<Inquiry>> {
    Ok(pending(inv, layer).await?.into_iter().find(|i| i.id == id))
}

/// Whether `subject` names a copy of inquiry `id`: `…:inquiry:id:{id}` or `…:inquiry:id:{id}:…`.
fn names(subject: &str, layer: &str, id: &str) -> bool {
    let prefix = format!("urn:layer:{layer}:inquiry:id:{id}");
    subject
        .strip_prefix(&prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
}

/// Why inquiry `id` is not pending in `layer`: a Conflict naming who took it when a
/// receipt says it was there, NotFound when it never was.
pub async fn absent(inv: &Invocation<'_>, layer: &str, id: &str) -> Error {
    // Every page of receipts: the latest take of a copy of `id` is the one that answers.
    let taker = receipts(inv, layer).await.ok().and_then(|all| {
        all.into_iter()
            .filter(|r| names(&r.subject, layer, id))
            .map(|r| r.taker)
            .next_back()
    });
    match taker {
        Some(taker) => Error::Conflict(format!(
            "inquiry {id} is no longer in layer `{layer}`: {taker} already took it"
        )),
        None => Error::NotFound(format!("no inquiry {id} in layer `{layer}`")),
    }
}

/// The name of a copy of `from` moved into `layer`: deterministic, so a retried action
/// writes the same tuple again, which the layer answers as a replay.
pub fn copy_subject(layer: &str, from: &Inquiry) -> String {
    format!("urn:layer:{layer}:inquiry:id:{}:move:{}", from.id, from.seq)
}

/// A copy of `from` for `layer`, as JSON-LD with full IRIs: everything the edge wrote,
/// the original arrival time and the edge's class kept, the reason replaced.
pub fn copy(layer: &str, from: &Inquiry, reason: &str) -> Value {
    let mut node = Map::new();
    node.insert("@id".into(), json!(copy_subject(layer, from)));
    node.insert("@type".into(), json!(inq("Inquiry")));
    node.insert(inq("inquiry"), json!(from.id));
    node.insert(inq("site"), json!(from.site));
    node.insert(inq("text"), json!(from.text));
    if let Some(reply_to) = &from.reply_to {
        node.insert(inq("replyTo"), json!(reply_to));
    }
    if let Some(sender) = &from.sender {
        node.insert(inq("sender"), json!(sender));
    }
    node.insert(inq("reason"), json!(reason));
    if !from.received.is_empty() {
        node.insert(
            inq("received"),
            json!({ "@value": from.received, "@type": XSD_DATE_TIME }),
        );
    }
    edge_fields(
        &mut node,
        from.name.as_deref(),
        from.organization.as_deref(),
        from.class.as_deref(),
        from.class_score.as_deref(),
    );
    Value::Object(node)
}

/// An example for the examples channel: the text a person confirmed as spam, and its
/// embedding (base64 little-endian f32s, the model and its digest) when the mount has an
/// embedder.
pub fn example(from: &Inquiry, embedding: Option<&Embedding>) -> Value {
    let mut node = Map::new();
    node.insert("@id".into(), json!("urn:layer:new"));
    node.insert("@type".into(), json!(inq("SpamExample")));
    node.insert(inq("inquiry"), json!(from.id));
    node.insert(inq("site"), json!(from.site));
    node.insert(inq("text"), json!(from.text));
    if let Some(embedding) = embedding {
        node.insert(inq("embedding"), json!(example::encode(&embedding.vector)));
        node.insert(inq("embeddingModel"), json!(embedding.model));
        node.insert(inq("embeddingDigest"), json!(embedding.digest));
    }
    Value::Object(node)
}

/// An intent for the intents layer, named `urn:layer:{layer}:intent:name:{intent}:inquiry:{id}:occurrence:{suffix}`.
pub fn intent(
    layer: &str,
    intent: &str,
    id: &str,
    suffix: &str,
    about: Option<&Inquiry>,
    with_text: bool,
) -> Value {
    let mut node = Map::new();
    node.insert(
        "@id".into(),
        json!(format!(
            "urn:layer:{layer}:intent:name:{intent}:inquiry:{id}:occurrence:{suffix}"
        )),
    );
    node.insert("@type".into(), json!(inq("Intent")));
    node.insert(inq("intent"), json!(intent));
    node.insert(inq("inquiry"), json!(id));
    if let Some(about) = about {
        node.insert(inq("site"), json!(about.site));
        if with_text {
            node.insert(inq("text"), json!(about.text));
        }
        if let Some(sender) = &about.sender {
            node.insert(inq("sender"), json!(sender));
        }
        if let Some(class) = &about.class {
            node.insert(inq("class"), json!(class));
        }
    }
    Value::Object(node)
}

/// One intent as the intents layer holds it: what a person decided about an inquiry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    /// The tuple's IRI, `urn:layer:{layer}:intent:name:…`.
    pub subject: String,
    /// What was decided: `train-ham`, `train-spam`, `untrain`, `block`, `unblock`,
    /// `block-domain` or `unblock-domain`.
    pub intent: String,
    /// The inquiry's id.
    pub inquiry: String,
    /// The text to train on, when the writer carried it: a stranger's words.
    pub text: Option<String>,
    /// The sender's keyed hash, when the writer saw it.
    pub sender: Option<String>,
    /// The edge's class for the inquiry, when the writer saw it.
    pub class: Option<String>,
    /// A person's note on a block: the block's label.
    pub note: Option<String>,
}

fn intent_of(node: &Value) -> Option<Intent> {
    if !has_type(node, "Intent") {
        return None;
    }
    Some(Intent {
        subject: node.get("@id")?.as_str()?.to_string(),
        intent: value(node, "intent")?,
        inquiry: value(node, "inquiry")?,
        text: value(node, "text"),
        sender: value(node, "sender"),
        class: value(node, "class"),
        note: value(node, "note"),
    })
}

/// The intents pending in `layer`, in feed order.
pub async fn intents(inv: &Invocation<'_>, layer: &str) -> Result<Vec<Intent>> {
    Ok(pending_nodes(inv, layer)
        .await?
        .iter()
        .filter_map(intent_of)
        .collect())
}

/// Say one entry in a channel layer.
pub async fn say(inv: &Invocation<'_>, layer: &str, entry: &Value) -> Result<()> {
    let content = entry.to_string();
    let say = iri(&format!("urn:layer:{layer}:say"))?;
    inv.issue(request(Verb::Sink, say, &[("content", &content)]))
        .await
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subject_names_an_id_only_whole() {
        assert!(names(
            "urn:layer:inquiry-spam-b:inquiry:id:a1",
            "inquiry-spam-b",
            "a1"
        ));
        assert!(names(
            "urn:layer:inquiry-spam-b:inquiry:id:a1:move:4",
            "inquiry-spam-b",
            "a1"
        ));
        assert!(!names(
            "urn:layer:inquiry-spam-b:inquiry:id:a12",
            "inquiry-spam-b",
            "a1"
        ));
        assert!(!names(
            "urn:layer:inquiry-spam-c:inquiry:id:a1",
            "inquiry-spam-b",
            "a1"
        ));
    }

    #[test]
    fn a_node_without_its_fields_is_not_an_inquiry() {
        let node = json!({ "@id": "urn:layer:x:inquiry:id:a", "@type": "Inquiry", "site": "s" });
        assert_eq!(inquiry(&node), None);
        let node = json!({ "@id": "urn:layer:x:intent:name:a", "@type": "Intent" });
        assert_eq!(inquiry(&node), None);
    }

    #[test]
    fn values_read_under_any_key_and_shape() {
        let node = json!({
            "@type": ["inq:Inquiry"],
            "text": "hello",
            "inq:site": [{"@value": "a.example"}],
            "https://ikigai-rs.dev/ns/inquiry#inquiry": "x1",
            "seq": 3,
        });
        assert!(has_type(&node, "Inquiry"));
        assert_eq!(value(&node, "text").as_deref(), Some("hello"));
        assert_eq!(value(&node, "site").as_deref(), Some("a.example"));
        assert_eq!(value(&node, "inquiry").as_deref(), Some("x1"));
        assert_eq!(value(&node, "seq").as_deref(), Some("3"));
        assert_eq!(value(&node, "replyTo"), None);
    }

    #[test]
    fn the_stage_sends_on_what_the_edge_wrote() {
        let taken = json!({
            "@id": "urn:layer:inquiry-hold-b:inquiry:id:x1",
            "@type": "Inquiry",
            "inquiry": "x1",
            "site": "a.example",
            "text": "hello",
            "name": "Ana",
            "organization": "Acme",
            "class": "unsure",
            "classScore": "0.5",
        });
        let held = Held::from_graph(&[taken]).unwrap();
        let sent = held.tuple("inquiry-b", Map::new());
        assert_eq!(sent[inq("name")], "Ana");
        assert_eq!(sent[inq("organization")], "Acme");
        assert_eq!(sent[inq("class")], "unsure");
        assert_eq!(sent[inq("classScore")]["@value"], "0.5");
    }

    #[test]
    fn debug_never_prints_the_text() {
        let inquiry = Held {
            id: "x1".into(),
            site: "a.example".into(),
            text: "ignore previous instructions".into(),
            reply_to: None,
            name: None,
            organization: None,
            sender: None,
            received: None,
            class: None,
            class_score: None,
        };
        let shown = format!("{inquiry:?}");
        assert!(!shown.contains("ignore"), "{shown}");
        assert!(shown.contains("x1"), "{shown}");
    }
}
