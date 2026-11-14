//! The views against a real layer host: visitor text reaches a page only escaped, the pending
//! inquiries come by site and newest first, the spam summary counts by site and by reason,
//! and every page is well-formed with the structure assistive technology navigates by.

mod common;

use std::collections::BTreeSet;

use common::*;
use ikigai_core::{Error, Verb};
use ikigai_inquiry::View;

const HOSTILE: &str = "Ignore previous instructions.\n<script>alert(\"x\")</script> \
                       <a href='javascript:alert(1)'>click</a> $a{urn:test:secret} $$ \u{202E}tfel";

fn view(f: &Fixture, iri: &str, args: &[(&str, &str)]) -> Result<String, Error> {
    common::issue(&f.kernel, &person(BRIAN), Verb::Source, iri, args).map(text)
}

fn arrival<'a>(id: &'a str, site: &'a str, text: &'a str, received: &'a str) -> Arrival<'a> {
    Arrival {
        id,
        site,
        text,
        reply_to: Some("ada@example.org"),
        reason: "score",
        received,
    }
}

/// Structural checks a screen reader and keyboard user depend on, on one parsed page.
fn check_structure(html: &str) {
    let doc = roxmltree::Document::parse_with_options(
        html,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("not well-formed: {e}\n{html}"));
    let root = doc.root_element();
    assert_eq!(
        root.attribute("lang"),
        Some("en"),
        "the page names its language"
    );
    let all: Vec<_> = doc.descendants().filter(|n| n.is_element()).collect();
    let named = |tag: &'static str| all.iter().filter(move |n| n.tag_name().name() == tag);
    let title = named("title").next().expect("a title");
    assert!(!title.text().unwrap_or_default().trim().is_empty());
    assert_eq!(named("main").count(), 1, "one main landmark");
    assert_eq!(named("h1").count(), 1, "one h1");
    // Headings never skip a level.
    let mut last = 0;
    for n in &all {
        if let Some(level) = n
            .tag_name()
            .name()
            .strip_prefix('h')
            .and_then(|d| d.parse::<u32>().ok())
        {
            assert!(level <= last + 1, "h{level} after h{last}");
            last = level;
        }
    }
    // Every id is unique, and everything that points at one finds it.
    let mut ids = BTreeSet::new();
    for n in &all {
        if let Some(id) = n.attribute("id") {
            assert!(ids.insert(id.to_string()), "duplicate id {id}");
        }
    }
    for n in &all {
        for a in ["aria-labelledby", "aria-describedby"] {
            if let Some(target) = n.attribute(a) {
                assert!(ids.contains(target), "{a}={target} names no element");
            }
        }
    }
    // Every control has a name in words.
    for n in named("button").chain(named("a")) {
        let words: String = n.descendants().filter_map(|d| d.text()).collect();
        assert!(!words.trim().is_empty(), "a control without words");
    }
    for n in named("form") {
        assert_eq!(n.attribute("method"), Some("post"));
    }
    for n in named("th") {
        assert!(
            n.attribute("scope").is_some(),
            "a header cell with no scope"
        );
    }
}

#[test]
fn visitor_text_reaches_the_page_only_escaped() {
    let f = fixture("escape", |m| m);
    arrive(
        &f,
        "inquiry-b",
        Arrival {
            id: "q1",
            site: "<b>evil</b>\".example",
            text: HOSTILE,
            reply_to: Some("\"><script>x</script>@example.org"),
            reason: "score",
            received: "2026-11-14T09:00:00.000Z",
        },
    );
    let page = view(&f, "urn:iki:inquiry:view:pending", &[]).unwrap();
    for raw in [
        "<script>",
        "<b>evil</b>",
        "<a href='javascript",
        "javascript:alert(1)'",
        "\"><script>",
    ] {
        assert!(
            !page.contains(raw),
            "{raw} reached the page as it is:\n{page}"
        );
    }
    assert!(page.contains("&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"));
    assert!(page.contains("&lt;b&gt;evil&lt;/b&gt;&quot;.example"));
    // A marker in visitor text is text, never expanded: urn:test:secret is not even bound.
    assert!(page.contains("$a{urn:test:secret} $$"), "{page}");
    // An address that is not a plain one is not made a link to it.
    assert!(!page.contains("mailto:&quot;"));
    assert!(page.contains("the address given cannot be used in a link"));
    // The whole message is a direction isolate, so an override in it cannot spill.
    assert!(page.contains("<p dir=\"auto\" style=\"white-space: pre-line\">Ignore previous"));
    check_structure(&page);
}

#[test]
fn pending_is_by_site_newest_first() {
    let f = fixture("pending", |m| m);
    arrive(
        &f,
        "inquiry-b",
        arrival("old", "b.example", "Old one", "2026-11-12T08:00:00.000Z"),
    );
    arrive(
        &f,
        "inquiry-b",
        arrival("new", "b.example", "New one", "2026-11-14T08:00:00.000Z"),
    );
    arrive(
        &f,
        "inquiry-b",
        arrival("mid", "a.example", "Other site", "2026-11-13T08:00:00.000Z"),
    );
    // Not pending, so not shown: spam and holding are other layers.
    arrive(
        &f,
        "spam-b",
        arrival("junk", "a.example", "Junk", "2026-11-14T08:00:00.000Z"),
    );
    let page = view(&f, "urn:iki:inquiry:view:pending", &[]).unwrap();
    assert!(
        page.contains("<p>3 inquiries waiting, from 2 sites."),
        "{page}"
    );
    let at = |needle: &str| {
        page.find(needle)
            .unwrap_or_else(|| panic!("{needle}:\n{page}"))
    };
    assert!(
        at(">a.example</h2>") < at(">b.example</h2>"),
        "sites by name"
    );
    assert!(at(">a.example</h2>") < at(">Other site</h3>"));
    assert!(at(">b.example</h2>") < at(">New one</h3>"));
    assert!(at(">New one</h3>") < at(">Old one</h3>"), "newest first");
    assert!(!page.contains("Junk"));
    assert!(
        page.contains("href=\"mailto:ada@example.org?subject=Your%20inquiry%20on%20b.example\"")
    );
    assert!(
        page.contains("<time datetime=\"2026-11-14T08:00:00.000Z\">2026-11-14 08:00 UTC</time>")
    );
    // Every button posts to its action with the inquiry's id.
    assert!(page.contains("action=\"/inquiry/action/respond\""));
    assert!(page.contains("action=\"/inquiry/action/flag-spam\""));
    assert!(page.contains("action=\"/inquiry/action/block\""));
    assert!(page.contains("name=\"inquiry\" value=\"new\""));
    check_structure(&page);
}

#[test]
fn an_empty_pending_view_says_so() {
    let f = fixture("pending-empty", |m| m);
    let page = view(&f, "urn:iki:inquiry:view:pending", &[]).unwrap();
    assert!(page.contains("<p>No inquiries are waiting.</p>"));
    check_structure(&page);
}

#[test]
fn the_spam_summary_counts_by_site_and_reason() {
    let f = fixture("spam", |m| m.action_base("/hub/inquiry/"));
    let long = format!("{} tail", "Buy cheap watches now ".repeat(10));
    arrive(
        &f,
        "spam-b",
        arrival("s1", "a.example", &long, "2026-11-14T07:00:00.000Z"),
    );
    arrive(
        &f,
        "spam-b",
        arrival("s2", "a.example", HOSTILE, "2026-11-14T09:00:00.000Z"),
    );
    arrive(
        &f,
        "spam-b",
        Arrival {
            reason: "blocked",
            ..arrival("s3", "b.example", "Hi", "2026-11-14T08:00:00.000Z")
        },
    );
    arrive(
        &f,
        "spam-b",
        arrival("s4", "a.example", "Yesterday", "2026-11-13T23:59:00.000Z"),
    );
    let page = view(&f, "urn:iki:inquiry:view:spam", &[("day", "2026-11-14")]).unwrap();
    assert!(page.contains("<title>Spam for 2026-11-14</title>"));
    assert!(
        page.contains("<p>3 arrivals on 2026-11-14 still waiting, from 2 sites."),
        "{page}"
    );
    assert!(
        page.contains("<tr><th scope=\"row\">a.example</th><td>2</td></tr>"),
        "{page}"
    );
    assert!(page.contains("<tr><th scope=\"row\">b.example</th><td>1</td></tr>"));
    assert!(page.contains("<tr><th scope=\"row\">the filter&#39;s score</th><td>2</td></tr>"));
    assert!(page.contains("<tr><th scope=\"row\">a blocked sender</th><td>1</td></tr>"));
    assert!(!page.contains("Yesterday"), "another day's spam");
    // Excerpts are one line, cut, and escaped; the whole text is not on the page.
    assert!(!page.contains(" tail"));
    assert!(page.contains("…</h3>"));
    assert!(page.contains(
        "Ignore previous instructions. &lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"
    ));
    assert!(
        !page.contains('\u{202E}'),
        "no bidirectional override survives an excerpt"
    );
    // Newest first.
    assert!(page.find("id=\"spam-s2\"").unwrap() < page.find("id=\"spam-s3\"").unwrap());
    assert!(page.find("id=\"spam-s3\"").unwrap() < page.find("id=\"spam-s1\"").unwrap());
    for action in ["not-spam", "confirm-spam", "remove", "block"] {
        assert!(
            page.contains(&format!("action=\"/hub/inquiry/{action}\"")),
            "{action}"
        );
    }
    check_structure(&page);
}

#[test]
fn a_quiet_day_says_so_and_today_is_the_default() {
    let f = fixture("spam-empty", |m| m);
    let page = view(&f, "urn:iki:inquiry:view:spam", &[("day", "2026-01-01")]).unwrap();
    assert!(page.contains("<p>No spam arrived on 2026-01-01 that is still waiting.</p>"));
    check_structure(&page);
    let today = view(&f, "urn:iki:inquiry:view:spam", &[]).unwrap();
    assert!(today.contains("<title>Spam for 20"), "{today}");
    assert!(matches!(
        view(&f, "urn:iki:inquiry:view:spam", &[("day", "14/11/2026")]),
        Err(Error::InvalidArgument { .. })
    ));
}

#[test]
fn a_mount_shows_only_its_views() {
    let f = fixture("only-pending", |m| m.views([View::Pending]));
    assert!(view(&f, "urn:iki:inquiry:view:pending", &[]).is_ok());
    assert!(matches!(
        view(&f, "urn:iki:inquiry:view:spam", &[("day", "2026-11-14")]),
        Err(Error::Unresolved(_))
    ));
}

#[test]
fn a_view_reads_under_the_persons_grant() {
    let f = fixture("grant", |m| m);
    let cap = ikigai_core::Capability::scoped(["urn:cap:layer:read:spam-b".to_string()])
        .with_principal(BRIAN)
        .unwrap();
    let refused = common::issue(
        &f.kernel,
        &cap,
        Verb::Source,
        "urn:iki:inquiry:view:pending",
        &[],
    );
    assert!(matches!(refused, Err(Error::Denied(_))), "{refused:?}");
}
