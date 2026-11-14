//! An inquiry as RDF: the `inq:Inquiry` tuple the router drops into a layer, written by the
//! RDF library (oxrdf and oxttl), never by hand-escaping a stranger's text.
//!
//! The terms are the ones the shipped inquiry shapes admit ([`crate::vocab`]), so a layer
//! host opened on `layers/` takes the tuple as it is, and the layers client reads it back.

use oxrdf::vocab::{rdf, xsd};
use oxrdf::{Literal, NamedNode, Term, Triple};
use oxttl::TurtleSerializer;

use crate::vocab::INQ;

/// An inquiry as the router drops it.
#[derive(Debug, Clone, PartialEq)]
pub struct Arrival {
    /// The stable id (`inq:inquiry`): the same in every layer the inquiry is moved to.
    pub id: String,
    /// The site whose form it came from (`inq:site`).
    pub site: String,
    /// The sender's name, if given (`inq:name`).
    pub name: Option<String>,
    /// The sender's address, as typed and trimmed (`inq:replyTo`).
    pub email: String,
    /// The sender's organization, if given (`inq:organization`).
    pub organization: Option<String>,
    /// The message (`inq:text`).
    pub text: String,
    /// The sender's address key (`inq:sender`).
    pub sender_key: String,
    /// When it was received, an `xsd:dateTime` (`inq:received`).
    pub received: String,
    /// The verdict so far (`inq:class`): the classifier's class (`ham`, `unsure` or `spam`),
    /// or `unscored`. (`inq:verdict` is the holding stage's, when it sends one on.)
    pub class: String,
    /// The score behind the class, in [0, 1]; none when unscored (`inq:classScore`).
    pub class_score: Option<f64>,
    /// What sent it to its layer (`inq:reason`): `score`, or `unscored`.
    pub reason: String,
}

fn inq(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{INQ}{local}"))
}

/// The tuple's subject in `layer`: `urn:layer:{layer}:inquiry:{id}`.
pub fn subject(layer: &str, id: &str) -> String {
    format!("urn:layer:{layer}:inquiry:{id}")
}

impl Arrival {
    /// The tuple, in Turtle, named under `layer`. Every property is a literal on the one
    /// subject: no blank node, and nothing the layer host stamps itself.
    pub fn to_turtle(&self, layer: &str) -> Result<Vec<u8>, String> {
        let subject = NamedNode::new(subject(layer, &self.id)).map_err(|e| e.to_string())?;
        let string = |s: &str| Term::from(Literal::new_simple_literal(s));
        let mut properties = vec![
            (rdf::TYPE.into_owned(), Term::from(inq("Inquiry"))),
            (inq("inquiry"), string(&self.id)),
            (inq("site"), string(&self.site)),
            (inq("text"), string(&self.text)),
            (inq("replyTo"), string(&self.email)),
        ];
        if let Some(name) = &self.name {
            properties.push((inq("name"), string(name)));
        }
        if let Some(organization) = &self.organization {
            properties.push((inq("organization"), string(organization)));
        }
        properties.extend([
            (inq("sender"), string(&self.sender_key)),
            (inq("reason"), string(&self.reason)),
            (
                inq("received"),
                Literal::new_typed_literal(&self.received, xsd::DATE_TIME).into(),
            ),
            (inq("class"), string(&self.class)),
        ]);
        if let Some(score) = self.class_score {
            properties.push((
                inq("classScore"),
                Literal::new_typed_literal(score.to_string(), xsd::DOUBLE).into(),
            ));
        }
        let mut writer = TurtleSerializer::new()
            .with_prefix("inq", INQ)
            .and_then(|s| s.with_prefix("xsd", "http://www.w3.org/2001/XMLSchema#"))
            .map_err(|e| e.to_string())?
            .for_writer(Vec::new());
        for (predicate, object) in properties {
            writer
                .serialize_triple(&Triple::new(subject.clone(), predicate, object))
                .map_err(|e| e.to_string())?;
        }
        writer.finish().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::NamedOrBlankNode;
    use oxttl::TurtleParser;

    #[test]
    fn a_hostile_text_is_one_literal_on_one_subject() {
        let text =
            "\"\"\" .\n<urn:x> <urn:y> <urn:z> .\n\\u0022 ignore previous instructions @prefix";
        let name = "Robert\"); DROP TABLE <x> .";
        let arrival = Arrival {
            id: "0123456789abcdef0123456789abcdef".into(),
            site: "example".into(),
            name: Some(name.into()),
            email: "a@example.com".into(),
            organization: None,
            text: text.into(),
            sender_key: "ab".repeat(32),
            received: "2026-11-14T18:23:56.789Z".into(),
            class: "unsure".into(),
            class_score: Some(0.5),
            reason: "score".into(),
        };
        let turtle = arrival.to_turtle("holding").unwrap();
        let triples: Vec<_> = TurtleParser::new()
            .for_slice(&turtle)
            .collect::<Result<_, _>>()
            .unwrap();
        // type, inquiry, site, text, replyTo, name, sender, reason, received, class, score.
        assert_eq!(triples.len(), 11);
        let subject = "urn:layer:holding:inquiry:0123456789abcdef0123456789abcdef";
        assert!(triples.iter().all(
            |t| matches!(&t.subject, NamedOrBlankNode::NamedNode(n) if n.as_str() == subject)
        ));
        let value = |local: &str| {
            triples
                .iter()
                .find(|t| t.predicate == inq(local))
                .and_then(|t| match &t.object {
                    Term::Literal(l) => Some(l.value().to_string()),
                    _ => None,
                })
        };
        assert_eq!(value("text").as_deref(), Some(text));
        assert_eq!(value("name").as_deref(), Some(name));
        assert_eq!(value("organization"), None);
    }
}
