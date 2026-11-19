//! The words an inquiry is written in, and the files a layer host opens to admit them.
//!
//! A deployment copies these beside its layer definitions: one context for every layer, the
//! inquiry shapes for the holding, spam and inquiry layers, the example shapes for the channel
//! of confirmed spam examples, and the intent shapes for the intents layer.

/// The namespace of the inquiry terms, `inq:`.
pub const INQ: &str = "https://ikigai-rs.dev/ns/inquiry#";

pub(crate) const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

pub(crate) const XSD_DATE_TIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";

/// The egress context of every inquiry layer: `layers/inquiry.context.jsonld`.
pub const CONTEXT: &str = include_str!("../layers/inquiry.context.jsonld");

/// The closed shapes of an inquiry, spam or holding layer: `layers/inquiry.shapes.ttl`.
pub const INQUIRY_SHAPES: &str = include_str!("../layers/inquiry.shapes.ttl");

/// The closed shapes of the examples channel, `inq:SpamExample` entries:
/// `layers/example.shapes.ttl`.
pub const EXAMPLE_SHAPES: &str = include_str!("../layers/example.shapes.ttl");

/// The closed shapes of the intents layer: `layers/intent.shapes.ttl`.
pub const INTENT_SHAPES: &str = include_str!("../layers/intent.shapes.ttl");

/// The `inq:` vocabulary as Turtle, `src/vocabulary.ttl`: an `owl:Ontology` at
/// `https://ikigai-rs.dev/ns/inquiry` defining every `inq:` term the layer files use. This is
/// the document served at that address, and its `owl:versionInfo` is this crate's version.
///
/// ```
/// let header = ikigai_inquiry::vocab::VOCABULARY
///     .lines()
///     .find(|line| line.trim_start().starts_with("owl:versionInfo"))
///     .unwrap();
/// assert_eq!(header, format!("    owl:versionInfo \"{}\" .", env!("CARGO_PKG_VERSION")));
/// ```
pub const VOCABULARY: &str = include_str!("vocabulary.ttl");

/// `inq:{local}`.
pub(crate) fn inq(local: &str) -> String {
    format!("{INQ}{local}")
}

/// Whether `id` is an inquiry id: what the shapes' `sh:pattern` admits.
pub fn is_inquiry_id(id: &str) -> bool {
    let mut chars = id.chars();
    id.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_follow_the_shape() {
        assert!(is_inquiry_id("01J9Z.a_b-c"));
        assert!(!is_inquiry_id(""));
        assert!(!is_inquiry_id("-a"));
        assert!(!is_inquiry_id("a:b"));
        assert!(!is_inquiry_id("a b"));
        assert!(!is_inquiry_id(&"a".repeat(65)));
        assert!(INQUIRY_SHAPES.contains(r#"sh:pattern "^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$""#));
    }
}
