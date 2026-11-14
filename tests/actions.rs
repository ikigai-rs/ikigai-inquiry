//! The actions against a real layer host: each move is OUT the new copy, THEN TAKE the old,
//! under the person's own grant; an action on an inquiry another action took is a Conflict.

mod common;

use common::*;
use ikigai_core::{Capability, Error, Verb};
use ikigai_inquiry::Mount;
use serde_json::Value;

fn arrival<'a>(id: &'a str, site: &'a str, text: &'a str) -> Arrival<'a> {
    Arrival {
        id,
        site,
        text,
        reply_to: Some("ada@example.org"),
        reason: "score",
        received: "2026-11-14T09:00:00.000Z",
    }
}

fn ids(nodes: &[Value]) -> Vec<String> {
    nodes
        .iter()
        .map(|n| n["inquiry"].as_str().unwrap().to_string())
        .collect()
}

fn intents(f: &Fixture) -> Vec<Value> {
    pending(f, "inquiry-intent-b")
}

#[test]
fn not_spam_moves_to_inquiry_and_asks_to_train_ham() {
    let f = fixture("not-spam", |m| m);
    arrive(
        &f,
        "inquiry-spam-b",
        arrival("a1", "a.example", "Hello, a real question."),
    );
    let said = act(&f, &person(BRIAN), "not-spam", "a1").unwrap();
    assert_eq!(said, "Moved inquiry a1 from spam to your inquiries.");

    assert!(pending(&f, "inquiry-spam-b").is_empty());
    let inquiry = pending(&f, "inquiry-b");
    assert_eq!(ids(&inquiry), ["a1"]);
    let copy = &inquiry[0];
    // Named by the copy it came from, and written by the person, not the edge.
    assert_eq!(copy["@id"], "urn:layer:inquiry-b:inquiry:id:a1:move:1");
    assert_eq!(copy["attributedTo"], BRIAN);
    assert_eq!(copy["reason"], "not-spam");
    assert_eq!(copy["site"], "a.example");
    assert_eq!(copy["text"], "Hello, a real question.");
    assert_eq!(copy["replyTo"], "ada@example.org");
    assert_eq!(copy["sender"], "hash-of-a1");
    // The edge's arrival time survives the move.
    assert_eq!(copy["received"], "2026-11-14T09:00:00.000Z");

    let intents = intents(&f);
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0]["intent"], "train-ham");
    assert_eq!(intents[0]["inquiry"], "a1");
    assert_eq!(intents[0]["text"], "Hello, a real question.");
    assert_eq!(intents[0]["sender"], "hash-of-a1");
}

#[test]
fn the_copy_is_out_before_the_old_is_taken() {
    // A person who may write to inquiry but not take from spam: the copy is written, the
    // take is refused, and the inquiry is in both layers, never in neither.
    let f = fixture("order", |m| m);
    arrive(&f, "inquiry-spam-b", arrival("a1", "a.example", "text"));
    let scopes: Vec<String> = person_scopes()
        .into_iter()
        .filter(|s| s != "urn:cap:layer:take:inquiry-spam-b")
        .collect();
    let cap = Capability::scoped(scopes).with_principal(BRIAN).unwrap();
    let refused = act(&f, &cap, "not-spam", "a1").unwrap_err();
    assert!(matches!(refused, Error::Denied(_)), "{refused:?}");
    assert_eq!(ids(&pending(&f, "inquiry-spam-b")), ["a1"]);
    assert_eq!(ids(&pending(&f, "inquiry-b")), ["a1"]);
    assert!(intents(&f).is_empty(), "no take, no intent");
}

#[test]
fn an_action_on_an_inquiry_another_took_is_a_conflict() {
    let f = fixture("conflict", |m| m);
    arrive(&f, "inquiry-spam-b", arrival("a1", "a.example", "text"));
    act(&f, &person(BRIAN), "confirm-spam", "a1").unwrap();
    // Chris, on a page loaded before Brian acted.
    let late = act(&f, &person(CHRIS), "not-spam", "a1").unwrap_err();
    match late {
        Error::Conflict(message) => assert!(message.contains(BRIAN), "{message}"),
        other => panic!("expected a Conflict, got {other:?}"),
    }
    // Nothing of the late action was written: no copy and no second intent.
    assert!(pending(&f, "inquiry-b").is_empty());
    let intents = intents(&f);
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0]["intent"], "train-spam");
    // The same action twice is a Conflict too.
    assert!(matches!(
        act(&f, &person(BRIAN), "remove", "a1"),
        Err(Error::Conflict(_))
    ));
}

#[test]
fn an_unknown_inquiry_is_not_found() {
    let f = fixture("unknown", |m| m);
    assert!(matches!(
        act(&f, &person(BRIAN), "not-spam", "nope"),
        Err(Error::NotFound(_))
    ));
    assert!(pending(&f, "inquiry-b").is_empty());
}

#[test]
fn an_id_that_is_not_one_is_refused() {
    let f = fixture("bad-id", |m| m);
    for bad in ["", "a:b", "a/../b", "<x>"] {
        match act(&f, &person(BRIAN), "remove", bad) {
            Err(Error::InvalidArgument { name, .. }) => assert_eq!(name, "inquiry"),
            other => panic!("{bad}: {other:?}"),
        }
    }
}

#[test]
fn confirm_spam_keeps_an_embedded_example() {
    let f = fixture("confirm", |m| {
        m.embedder(ikigai_core::Iri::parse(EMBEDDER).unwrap())
    });
    arrive(&f, "inquiry-spam-b", arrival("s1", "b.example", "Buy now"));
    let said = act(&f, &person(BRIAN), "confirm-spam", "s1").unwrap();
    assert_eq!(
        said,
        "Confirmed inquiry s1 as spam and kept it as an example."
    );
    assert!(pending(&f, "inquiry-spam-b").is_empty());
    let examples = examples(&f);
    assert_eq!(examples.len(), 1);
    assert_eq!(examples[0]["@type"], "SpamExample");
    assert_eq!(examples[0]["inquiry"], "s1");
    assert_eq!(examples[0]["site"], "b.example");
    assert_eq!(examples[0]["text"], "Buy now");
    // What the holding stage compares with: base64 little-endian f32s, the model, the digest.
    let vector = ikigai_inquiry::example::decode(examples[0]["embedding"].as_str().unwrap());
    assert_eq!(vector, Some(vec![7.0, 1.0]));
    assert_eq!(examples[0]["embeddingModel"], "embeddinggemma:latest");
    assert_eq!(examples[0]["embeddingDigest"], EMBED_DIGEST);
    assert_eq!(examples[0]["attributedTo"], BRIAN);
    let intents = intents(&f);
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0]["intent"], "train-spam");
    assert_eq!(intents[0]["text"], "Buy now");
}

#[test]
fn confirm_spam_without_an_embedder_keeps_the_text() {
    let f = fixture("confirm-plain", |m| m);
    arrive(&f, "inquiry-spam-b", arrival("s1", "b.example", "Buy now"));
    act(&f, &person(BRIAN), "confirm-spam", "s1").unwrap();
    let examples = examples(&f);
    assert_eq!(examples.len(), 1);
    for term in ["embedding", "embeddingModel", "embeddingDigest"] {
        assert!(examples[0].get(term).is_none(), "{term}");
    }
}

#[test]
fn remove_takes_from_spam_and_nothing_else() {
    let f = fixture("remove", |m| m);
    arrive(&f, "inquiry-spam-b", arrival("s1", "b.example", "Buy now"));
    assert_eq!(
        act(&f, &person(BRIAN), "remove", "s1").unwrap(),
        "Removed inquiry s1 from spam."
    );
    assert!(pending(&f, "inquiry-spam-b").is_empty());
    assert!(pending(&f, "inquiry-b").is_empty());
    assert!(examples(&f).is_empty());
    assert!(intents(&f).is_empty());
}

#[test]
fn flag_spam_moves_to_spam_and_asks_to_train_spam() {
    let f = fixture("flag", |m| m);
    arrive(&f, "inquiry-b", arrival("q1", "a.example", "Cheap pills"));
    act(&f, &person(BRIAN), "flag-spam", "q1").unwrap();
    assert!(pending(&f, "inquiry-b").is_empty());
    let spam = pending(&f, "inquiry-spam-b");
    assert_eq!(ids(&spam), ["q1"]);
    assert_eq!(spam[0]["reason"], "flagged");
    assert_eq!(intents(&f)[0]["intent"], "train-spam");

    // And back again: a fresh copy each way, never a name used before.
    act(&f, &person(BRIAN), "not-spam", "q1").unwrap();
    let back = pending(&f, "inquiry-b");
    assert_eq!(ids(&back), ["q1"]);
    assert_ne!(back[0]["@id"], "urn:layer:inquiry-b:inquiry:id:q1");
}

#[test]
fn respond_marks_handled() {
    let f = fixture("respond", |m| m);
    arrive(&f, "inquiry-b", arrival("q1", "a.example", "Question"));
    assert_eq!(
        act(&f, &person(BRIAN), "respond", "q1").unwrap(),
        "Marked inquiry q1 handled."
    );
    assert!(pending(&f, "inquiry-b").is_empty());
    assert!(matches!(
        act(&f, &person(CHRIS), "respond", "q1"),
        Err(Error::Conflict(_))
    ));
}

#[test]
fn block_and_unblock_are_intents_for_the_edge() {
    let f = fixture("block", |m| m);
    arrive(&f, "inquiry-spam-b", arrival("s1", "b.example", "Buy now"));
    assert_eq!(
        act(&f, &person(BRIAN), "block", "s1").unwrap(),
        "Asked to block the sender of inquiry s1."
    );
    // The inquiry stays where it is.
    assert_eq!(ids(&pending(&f, "inquiry-spam-b")), ["s1"]);
    act(&f, &person(BRIAN), "remove", "s1").unwrap();
    // Unblocking works after the inquiry is gone: the edge knows the id.
    act(&f, &person(BRIAN), "unblock", "s1").unwrap();
    let intents = intents(&f);
    assert_eq!(intents.len(), 2);
    assert_eq!(intents[0]["intent"], "block");
    assert_eq!(intents[0]["sender"], "hash-of-s1");
    assert_eq!(intents[0]["site"], "b.example");
    assert!(intents[0].get("text").is_none(), "a block carries no text");
    assert_eq!(intents[1]["intent"], "unblock");
    assert_eq!(intents[1]["inquiry"], "s1");
}

#[test]
fn only_the_acting_identity_acts() {
    let f = fixture("acting", |m: Mount| m.acting_as(BRIAN));
    arrive(&f, "inquiry-spam-b", arrival("s1", "b.example", "Buy now"));
    assert!(matches!(
        act(&f, &person(CHRIS), "remove", "s1"),
        Err(Error::Denied(_))
    ));
    assert!(matches!(
        common::issue(
            &f.kernel,
            &person(CHRIS),
            Verb::Source,
            "urn:iki:inquiry:view:name:pending",
            &[]
        ),
        Err(Error::Denied(_))
    ));
    act(&f, &person(BRIAN), "remove", "s1").unwrap();
}

#[test]
fn every_action_is_a_sink() {
    let f = fixture("sinks", |m| m);
    arrive(&f, "inquiry-spam-b", arrival("s1", "b.example", "Buy now"));
    let sourced = common::issue(
        &f.kernel,
        &person(BRIAN),
        Verb::Source,
        "urn:iki:inquiry:action:name:remove",
        &[("inquiry", "s1")],
    );
    assert!(
        matches!(
            sourced,
            Err(Error::Unsupported {
                verb: Some(Verb::Source),
                ..
            })
        ),
        "{sourced:?}"
    );
    assert_eq!(ids(&pending(&f, "inquiry-spam-b")), ["s1"]);
}
