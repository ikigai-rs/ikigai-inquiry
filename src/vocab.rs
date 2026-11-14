//! The words an inquiry is written in, and the files a layer host opens to admit them.
//!
//! A deployment copies these beside its layer definitions: one context for every layer, and
//! one shapes file per kind of layer (inquiry, spam and holding share [`INQUIRY_SHAPES`]).

/// The namespace of the inquiry terms, `inq:`.
pub const INQ: &str = "https://ikigai-rs.dev/ns/inquiry#";

pub(crate) const XSD_DATE_TIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";

/// The egress context of every inquiry layer: `layers/inquiry.context.jsonld`.
pub const CONTEXT: &str = include_str!("../layers/inquiry.context.jsonld");

/// The closed shapes of an inquiry, spam or holding layer: `layers/inquiry.shapes.ttl`.
pub const INQUIRY_SHAPES: &str = include_str!("../layers/inquiry.shapes.ttl");

/// The closed shapes of the examples channel: `layers/example.shapes.ttl`.
pub const EXAMPLE_SHAPES: &str = include_str!("../layers/example.shapes.ttl");

/// The closed shapes of the intents layer: `layers/intent.shapes.ttl`.
pub const INTENT_SHAPES: &str = include_str!("../layers/intent.shapes.ttl");

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
