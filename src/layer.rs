//! Talking to the layer doors: every read and write here is a sub-request under the
//! caller's own capability, so a person can do through the client exactly what their grant
//! lets them do at the doors, and nothing more.

use ikigai_core::{ArgRef, Error, Invocation, Iri, Request, Result, Verb};
use serde_json::{json, Map, Value};

use crate::vocab::{INQ, XSD_DATE_TIME};

/// One inquiry as a layer holds it: a tuple in an inquiry, spam or holding layer.
///
/// `text` and `reply_to` are a stranger's words. They are carried, compared and escaped,
/// never interpreted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inquiry {
    /// The tuple's IRI, `urn:layer:{layer}:inquiry:…`.
    pub subject: String,
    /// The inquiry's id, the same in every layer a move takes it to.
    pub id: String,
    /// The site whose form it came from.
    pub site: String,
    /// What the visitor wrote.
    pub text: String,
    /// The address the visitor gave for a reply, as typed.
    pub reply_to: Option<String>,
    /// The edge's keyed hash of the sender.
    pub sender: Option<String>,
    /// What sent it to this layer.
    pub reason: Option<String>,
    /// When the edge received it (`xsd:dateTime`), or when the layer stamped this copy
    /// when the edge did not say.
    pub received: String,
    /// This copy's place in its layer's feed.
    pub seq: u64,
}

/// `text` as an IRI; one this module built that does not parse is its own defect.
pub(crate) fn iri(text: &str) -> Result<Iri> {
    Iri::parse(text).map_err(|e| Error::Endpoint(format!("`{text}` is not an IRI: {e}")))
}

/// `urn:layer:{layer}:tuples`.
fn tuples(layer: &str) -> Result<Iri> {
    iri(&format!("urn:layer:{layer}:tuples"))
}

fn request(verb: Verb, iri: Iri, args: &[(&str, &str)]) -> Request {
    let mut request = Request::new(verb, iri);
    for (name, value) in args {
        request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    request
}

fn json_answer(bytes: &[u8], what: &str) -> Result<Value> {
    serde_json::from_slice(bytes)
        .map_err(|e| Error::Endpoint(format!("{what}: the layer's answer is not JSON: {e}")))
}

/// A node's value for `term` as one string: a layer's JSON face gives a literal as its
/// lexical string, and a value that is anything else is treated as absent.
fn text(node: &Value, term: &str) -> Option<String> {
    match node.get(term)? {
        Value::String(s) => Some(s.clone()),
        Value::Array(values) => values.first().and_then(|v| v.as_str()).map(str::to_string),
        _ => None,
    }
}

fn is_inquiry(node: &Value) -> bool {
    match node.get("@type") {
        Some(Value::String(t)) => t == "Inquiry",
        Some(Value::Array(types)) => types.iter().any(|t| t == "Inquiry"),
        _ => false,
    }
}

fn inquiry(node: &Value) -> Option<Inquiry> {
    if !is_inquiry(node) {
        return None;
    }
    let created = text(node, "created");
    Some(Inquiry {
        subject: node.get("@id")?.as_str()?.to_string(),
        id: text(node, "inquiry")?,
        site: text(node, "site")?,
        text: text(node, "text")?,
        reply_to: text(node, "replyTo"),
        sender: text(node, "sender"),
        reason: text(node, "reason"),
        received: text(node, "received").or(created).unwrap_or_default(),
        seq: text(node, "seq")?.parse().ok()?,
    })
}

/// The inquiries pending in `layer`, in feed order.
pub async fn pending(inv: &Invocation<'_>, layer: &str) -> Result<Vec<Inquiry>> {
    let answer = inv
        .issue(request(
            Verb::Source,
            tuples(layer)?,
            &[("as", "application/ld+json")],
        ))
        .await?;
    let doc = json_answer(&answer.bytes, layer)?;
    Ok(doc
        .get("@graph")
        .and_then(Value::as_array)
        .map(|nodes| nodes.iter().filter_map(inquiry).collect())
        .unwrap_or_default())
}

/// The pending copy of inquiry `id` in `layer`, if there is one.
pub async fn find(inv: &Invocation<'_>, layer: &str, id: &str) -> Result<Option<Inquiry>> {
    Ok(pending(inv, layer).await?.into_iter().find(|i| i.id == id))
}

/// Whether `subject` names a copy of inquiry `id`: `…:inquiry:{id}` or `…:inquiry:{id}:…`.
fn names(subject: &str, layer: &str, id: &str) -> bool {
    let prefix = format!("urn:layer:{layer}:inquiry:{id}");
    subject
        .strip_prefix(&prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
}

/// Why inquiry `id` is not pending in `layer`: a Conflict naming who took it when a
/// receipt says it was there, NotFound when it never was.
pub async fn absent(inv: &Invocation<'_>, layer: &str, id: &str) -> Error {
    let receipts = inv
        .issue(request(
            Verb::Source,
            match tuples(layer) {
                Ok(iri) => iri,
                Err(e) => return e,
            },
            &[("state", "taken"), ("as", "application/json")],
        ))
        .await
        .and_then(|answer| json_answer(&answer.bytes, layer));
    let taker = receipts.ok().and_then(|doc| {
        doc.get("receipts")?
            .as_array()?
            .iter()
            .filter(|r| r["subject"].as_str().is_some_and(|s| names(s, layer, id)))
            .filter_map(|r| r["taker"].as_str().map(str::to_string))
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
    format!("urn:layer:{layer}:inquiry:{}:m{}", from.id, from.seq)
}

fn inq(term: &str) -> String {
    format!("{INQ}{term}")
}

/// A copy of `from` for `layer`, as JSON-LD with full IRIs: everything the edge wrote,
/// the original arrival time kept, the reason replaced.
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
    Value::Object(node)
}

/// An example for the examples channel: the text a person confirmed as spam.
pub fn example(from: &Inquiry, embedding: Option<String>) -> Value {
    let mut node = Map::new();
    node.insert("@id".into(), json!("urn:layer:new"));
    node.insert("@type".into(), json!(inq("Example")));
    node.insert(inq("inquiry"), json!(from.id));
    node.insert(inq("site"), json!(from.site));
    node.insert(inq("text"), json!(from.text));
    if let Some(sender) = &from.sender {
        node.insert(inq("sender"), json!(sender));
    }
    if let Some(embedding) = embedding {
        node.insert(inq("embedding"), json!(embedding));
    }
    Value::Object(node)
}

/// An intent for the intents layer, named `urn:layer:{layer}:intent:{intent}:{id}:{suffix}`.
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
        json!(format!("urn:layer:{layer}:intent:{intent}:{id}:{suffix}")),
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
    }
    Value::Object(node)
}

/// OUT: drop one tuple into a tuplespace layer.
pub async fn out(inv: &Invocation<'_>, layer: &str, tuple: &Value) -> Result<()> {
    let content = tuple.to_string();
    inv.issue(request(
        Verb::Sink,
        tuples(layer)?,
        &[("content", &content)],
    ))
    .await
    .map(drop)
}

/// TAKE: remove one tuple, atomically. A tuple another taker took first is the layer's
/// Conflict, naming the winner.
pub async fn take(inv: &Invocation<'_>, layer: &str, subject: &str) -> Result<()> {
    inv.issue(request(Verb::Delete, tuples(layer)?, &[("tuple", subject)]))
        .await
        .map(drop)
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
        assert!(names("urn:layer:spam-b:inquiry:a1", "spam-b", "a1"));
        assert!(names("urn:layer:spam-b:inquiry:a1:m4", "spam-b", "a1"));
        assert!(!names("urn:layer:spam-b:inquiry:a12", "spam-b", "a1"));
        assert!(!names("urn:layer:spam-c:inquiry:a1", "spam-b", "a1"));
    }

    #[test]
    fn a_node_without_its_fields_is_not_an_inquiry() {
        let node = json!({ "@id": "urn:layer:x:inquiry:a", "@type": "Inquiry", "site": "s" });
        assert_eq!(inquiry(&node), None);
        let node = json!({ "@id": "urn:layer:x:intent:a", "@type": "Intent" });
        assert_eq!(inquiry(&node), None);
    }
}
