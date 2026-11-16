//! The views: `urn:iki:inquiry:view:name:pending` and `urn:iki:inquiry:view:name:spam`, each a whole
//! HTML page.
//!
//! Every piece of visitor text, and everything the module writes from it (an excerpt, a
//! reply link, a site's name), reaches a page through `$h{…}` in a template that ikigai-fn's
//! `compose` or `each` fills, and nowhere else: the Rust here writes only the page's frame,
//! from words of its own and numbers. The data endpoints answer flat JSON rows for `each`;
//! the templates are fixed when the space is built, with the mount's action base in them.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use ikigai_core::{
    ArgRef, ArgSpec, Description, Endpoint, Error, FnEndpoint, Invocation, ReprType,
    Representation, Request, Result, Verb,
};
use ikigai_fn::html::{attr, html_text};
use serde_json::{json, Value};

use crate::action::{check_acting, Action};
use crate::layer::{self, Inquiry};
use crate::mount::Mount;
use crate::text;

pub(crate) const EACH: &str = "urn:iki:inquiry:function:name:each";
pub(crate) const PENDING_SECTION: &str = "urn:iki:inquiry:template:name:pending-section-rendered";
pub(crate) const SPAM_BODY: &str = "urn:iki:inquiry:template:name:spam-body-rendered";
pub(crate) const PENDING_SECTION_TEMPLATE: &str = "urn:iki:inquiry:template:name:pending-section";
pub(crate) const SPAM_BODY_TEMPLATE: &str = "urn:iki:inquiry:template:name:spam-body";

/// The templates, by IRI: `(iri, endpoint name, text)`.
pub(crate) fn templates(mount: &Mount) -> Vec<(&'static str, &'static str, String)> {
    let form = |action: Action, label: &str, prefix: &str| {
        let target =
            ikigai_fn::escape_markers(&attr(&format!("{}{}", mount.action_base, action.name())));
        format!(
            "<form method=\"post\" action=\"{target}\">\
             <input type=\"hidden\" name=\"inquiry\" value=\"$h{{{{id}}}}\" />\
             <button type=\"submit\" aria-describedby=\"{prefix}-$h{{{{id}}}}\">{label}</button>\
             </form>"
        )
    };
    let pending_section = format!(
        "<section aria-labelledby=\"site-$h{{{{key}}}}\">\n\
         <h2 id=\"site-$h{{{{key}}}}\">$h{{{{site}}}}</h2>\n\
         <ol>\n$r{{{EACH}?src=urn:iki:inquiry:data:pending:site:{{key}}&template=urn:iki:inquiry:template:name:pending-row}}</ol>\n\
         </section>\n"
    );
    let pending_row = format!(
        "<li><article aria-labelledby=\"inquiry-$h{{{{id}}}}\">\n\
         <h3 id=\"inquiry-$h{{{{id}}}}\">$h{{{{excerpt}}}}</h3>\n\
         <p>Received <time datetime=\"$h{{{{received}}}}\">$h{{{{when}}}}</time>. \
         Reply address: $h{{{{reply}}}}.</p>\n\
         <details><summary>Full message</summary>\
         <p dir=\"auto\" class=\"inquiry-message\">$h{{{{text}}}}</p></details>\n\
         <p><a href=\"$h{{{{mailto}}}}\" aria-describedby=\"inquiry-$h{{{{id}}}}\">$h{{{{reply_label}}}}</a></p>\n\
         <div role=\"group\" aria-label=\"Actions\">\n{}\n{}\n{}\n</div>\n\
         </article></li>\n",
        form(Action::Respond, "Mark handled", "inquiry"),
        form(Action::FlagSpam, "Move to spam", "inquiry"),
        form(Action::Block, "Block sender", "inquiry"),
    );
    let count_table = |part: &str, heading: &str, column: &str| {
        format!(
            "<section aria-labelledby=\"by-{part}\">\n<h2 id=\"by-{part}\">{heading}</h2>\n\
             <table aria-labelledby=\"by-{part}\">\n<thead><tr><th scope=\"col\">{column}</th>\
             <th scope=\"col\">Arrivals</th></tr></thead>\n<tbody>\n\
             $r{{{EACH}?src=urn:iki:inquiry:data:spam:date:{{day}}:part:{part}&template=urn:iki:inquiry:template:name:count-row}}\
             </tbody>\n</table>\n</section>\n"
        )
    };
    let spam_body = format!(
        "{}{}<section aria-labelledby=\"arrivals\">\n<h2 id=\"arrivals\">Arrivals</h2>\n<ol>\n\
         $r{{{EACH}?src=urn:iki:inquiry:data:spam:date:{{day}}:part:item&template=urn:iki:inquiry:template:name:spam-row}}\
         </ol>\n</section>\n",
        count_table("site", "Arrivals by site", "Site"),
        count_table("reason", "Arrivals by what sent them to spam", "Sent by"),
    );
    let spam_row = format!(
        "<li><article aria-labelledby=\"spam-$h{{{{id}}}}\">\n\
         <h3 id=\"spam-$h{{{{id}}}}\">$h{{{{excerpt}}}}</h3>\n\
         <p>Site: $h{{{{site}}}}. Sent here by: $h{{{{reason}}}}. \
         Received <time datetime=\"$h{{{{received}}}}\">$h{{{{when}}}}</time>.</p>\n\
         <div role=\"group\" aria-label=\"Actions\">\n{}\n{}\n{}\n{}\n</div>\n\
         </article></li>\n",
        form(Action::NotSpam, "Not spam", "spam"),
        form(Action::ConfirmSpam, "Confirm as spam", "spam"),
        form(Action::Remove, "Remove", "spam"),
        form(Action::Block, "Block sender", "spam"),
    );
    let count_row = "<tr><th scope=\"row\">$h{{label}}</th><td>$h{{count}}</td></tr>\n".to_string();
    vec![
        (
            PENDING_SECTION_TEMPLATE,
            "inquiry-ui-pending-section",
            pending_section,
        ),
        (
            "urn:iki:inquiry:template:name:pending-row",
            "inquiry-ui-pending-row",
            pending_row,
        ),
        (SPAM_BODY_TEMPLATE, "inquiry-ui-spam-body", spam_body),
        (
            "urn:iki:inquiry:template:name:spam-row",
            "inquiry-ui-spam-row",
            spam_row,
        ),
        (
            "urn:iki:inquiry:template:name:count-row",
            "inquiry-ui-count-row",
            count_row,
        ),
    ]
}

/// A template's text as a cacheable HTML resource.
pub(crate) fn template(name: &'static str, text: String) -> FnEndpoint {
    FnEndpoint::new(name, move |_| {
        Ok(Representation::new(ReprType::new("text/html"), text.clone().into_bytes()).cacheable())
    })
}

fn rows(rows: Vec<Value>) -> Representation {
    Representation::new(
        ReprType::new("application/json"),
        Value::Array(rows).to_string().into_bytes(),
    )
}

fn newest_first(mut inquiries: Vec<Inquiry>) -> Vec<Inquiry> {
    inquiries.sort_by(|a, b| b.received.cmp(&a.received).then(b.seq.cmp(&a.seq)));
    inquiries
}

fn excerpt(inquiry: &Inquiry) -> String {
    let line = text::excerpt(&inquiry.text);
    if line.is_empty() {
        "(no text)".to_string()
    } else {
        line
    }
}

fn pending_row(inquiry: &Inquiry) -> Value {
    let given = inquiry.reply_to.as_deref().unwrap_or_default();
    let link = text::mailto(given, &inquiry.site);
    let (mailto, reply_label) = match &link {
        Some(link) => (link.clone(), "Write a reply".to_string()),
        None => (
            format!(
                "mailto:?subject={}",
                ikigai_fn::html::url_segment(&format!("Your inquiry on {}", inquiry.site))
            ),
            "Write a reply (the address given cannot be used in a link)".to_string(),
        ),
    };
    json!({
        "id": inquiry.id,
        "excerpt": excerpt(inquiry),
        "text": inquiry.text,
        "received": inquiry.received,
        "when": text::when(&inquiry.received),
        "reply": if given.is_empty() { "none given" } else { given },
        "mailto": mailto,
        "reply_label": reply_label,
    })
}

/// The pending inquiries of one site, newest first: `urn:iki:inquiry:data:pending:site:{key}`.
pub(crate) struct PendingData(pub(crate) Arc<Mount>);

#[async_trait]
impl Endpoint for PendingData {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        check_acting(&self.0, inv, Verb::Source)?;
        let key = inv.bindings.get("key").unwrap_or_default();
        let site = text::unkey(key).ok_or_else(|| Error::InvalidArgument {
            name: "key".to_string(),
            detail: "not a site key".to_string(),
        })?;
        let all = layer::pending(inv, &self.0.layers.inquiry).await?;
        let mine = all.into_iter().filter(|i| i.site == site).collect();
        Ok(rows(newest_first(mine).iter().map(pending_row).collect()))
    }

    fn name(&self) -> &str {
        "inquiry-data-pending"
    }
}

/// A day's spam, as rows: `urn:iki:inquiry:data:spam:date:{day}:part:{part}`, `part` one of `site`,
/// `reason` (label and count, most first) or `item` (newest first).
pub(crate) struct SpamData(pub(crate) Arc<Mount>);

async fn days_spam(inv: &Invocation<'_>, mount: &Mount, day: &str) -> Result<Vec<Inquiry>> {
    let all = layer::pending(inv, &mount.layers.spam).await?;
    Ok(newest_first(
        all.into_iter()
            .filter(|i| i.received.starts_with(day))
            .collect(),
    ))
}

fn counts(labels: impl Iterator<Item = String>) -> Vec<Value> {
    let mut by: BTreeMap<String, usize> = BTreeMap::new();
    for label in labels {
        *by.entry(label).or_default() += 1;
    }
    let mut counted: Vec<(String, usize)> = by.into_iter().collect();
    counted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    counted
        .into_iter()
        .map(|(label, count)| json!({ "label": label, "count": count.to_string() }))
        .collect()
}

#[async_trait]
impl Endpoint for SpamData {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        check_acting(&self.0, inv, Verb::Source)?;
        let day = inv.bindings.get("day").unwrap_or_default();
        check_day(day)?;
        let spam = days_spam(inv, &self.0, day).await?;
        match inv.bindings.get("part").unwrap_or_default() {
            "site" => Ok(rows(counts(spam.iter().map(|i| i.site.clone())))),
            "reason" => Ok(rows(counts(
                spam.iter().map(|i| text::reason(i.reason.as_deref())),
            ))),
            "item" => Ok(rows(
                spam.iter()
                    .map(|i| {
                        json!({
                            "id": i.id,
                            "excerpt": excerpt(i),
                            "site": i.site,
                            "reason": text::reason(i.reason.as_deref()),
                            "received": i.received,
                            "when": text::when(&i.received),
                        })
                    })
                    .collect(),
            )),
            other => Err(Error::NotFound(format!(
                "no spam summary part `{other}`: site, reason or item"
            ))),
        }
    }

    fn name(&self) -> &str {
        "inquiry-data-spam"
    }
}

fn check_day(day: &str) -> Result<()> {
    let b = day.as_bytes();
    let ok = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && [0, 1, 2, 3, 5, 6, 8, 9]
            .iter()
            .all(|&i| b[i].is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidArgument {
            name: "day".to_string(),
            detail: "a day is YYYY-MM-DD".to_string(),
        })
    }
}

/// The UTC day `millis` after the epoch falls on, as `YYYY-MM-DD` (Howard Hinnant's
/// civil-from-days).
pub(crate) fn utc_day(millis: u64) -> String {
    let days = (millis / 86_400_000) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn page(title: &str, body: &str) -> Representation {
    let title = html_text(title);
    let html = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\" />\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n\
         <title>{title}</title>\n</head>\n<body>\n<main>\n<h1>{title}</h1>\n{body}</main>\n\
         </body>\n</html>\n"
    );
    Representation::new(ReprType::new("text/html"), html.into_bytes())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

async fn fill(inv: &Invocation<'_>, iri: &str, args: &[(&str, &str)]) -> Result<String> {
    let mut request = Request::new(Verb::Source, layer::iri(iri)?);
    for (name, value) in args {
        request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    let filled = inv.issue(request).await?;
    String::from_utf8(filled.bytes)
        .map_err(|_| Error::Endpoint(format!("`{iri}` did not answer UTF-8 text")))
}

/// `urn:iki:inquiry:view:name:pending`: the pending inquiries, a section per site (by name), each
/// newest first.
pub(crate) struct PendingView(pub(crate) Arc<Mount>);

#[async_trait]
impl Endpoint for PendingView {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        check_acting(&self.0, inv, Verb::Source)?;
        let all = layer::pending(inv, &self.0.layers.inquiry).await?;
        let mut sites: Vec<&str> = all.iter().map(|i| i.site.as_str()).collect();
        sites.sort_unstable();
        sites.dedup();
        let mut body = if all.is_empty() {
            "<p>No inquiries are waiting.</p>\n".to_string()
        } else {
            format!(
                "<p>{} waiting, from {}. Newest first within each site.</p>\n",
                plural(all.len(), "inquiry", "inquiries"),
                plural(sites.len(), "site", "sites"),
            )
        };
        for site in sites {
            let key = text::key(site);
            body.push_str(&fill(inv, PENDING_SECTION, &[("key", &key), ("site", site)]).await?);
        }
        Ok(page("Inquiries", &body))
    }

    fn name(&self) -> &str {
        "inquiry-view-pending"
    }

    fn describe(&self) -> Description {
        Description::new(self.name())
            .title("Pending inquiries")
            .summary(
                "The inquiries waiting in the mount's inquiry layer, a section per site, each \
                 newest first, with a reply link and a button per action.",
            )
            .verb(Verb::Source)
            .output("text/html")
    }
}

/// `urn:iki:inquiry:view:name:spam`: one day's spam (`day=YYYY-MM-DD`, today in UTC by the kernel
/// clock when absent).
pub(crate) struct SpamView(pub(crate) Arc<Mount>);

#[async_trait]
impl Endpoint for SpamView {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        check_acting(&self.0, inv, Verb::Source)?;
        let day = match inv.inline_str("day") {
            Ok(day) => day.to_string(),
            Err(Error::MissingArgument(_)) => utc_day(
                inv.now()
                    .ok_or_else(|| {
                        Error::Unavailable(
                            "the kernel has no clock: name the day with `day=`".to_string(),
                        )
                    })?
                    .as_millis(),
            ),
            Err(e) => return Err(e),
        };
        check_day(&day)?;
        let spam = days_spam(inv, &self.0, &day).await?;
        let title = format!("Spam for {day}");
        let body = if spam.is_empty() {
            format!("<p>No spam arrived on {day} that is still waiting.</p>\n")
        } else {
            let sites = {
                let mut s: Vec<&str> = spam.iter().map(|i| i.site.as_str()).collect();
                s.sort_unstable();
                s.dedup();
                s.len()
            };
            format!(
                "<p>{} on {day} still waiting, from {}.</p>\n{}",
                plural(spam.len(), "arrival", "arrivals"),
                plural(sites, "site", "sites"),
                fill(inv, SPAM_BODY, &[("day", &day)]).await?
            )
        };
        Ok(page(&title, &body))
    }

    fn name(&self) -> &str {
        "inquiry-view-spam"
    }

    fn describe(&self) -> Description {
        Description::new(self.name())
            .title("The day's spam")
            .summary(
                "One day's spam still waiting in the mount's spam layer: arrivals by site and by \
                 what sent them there, and each arrival with a one-line excerpt and a button \
                 per action.",
            )
            .verb(Verb::Source)
            .input(
                ArgSpec::new("day")
                    .summary("the day, YYYY-MM-DD; today in UTC when absent")
                    .optional(),
            )
            .output("text/html")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_utc_calendar_days() {
        assert_eq!(utc_day(0), "1970-01-01");
        assert_eq!(utc_day(951_782_400_000), "2000-02-29");
        assert_eq!(utc_day(1_794_700_800_000 - 1), "2026-11-14");
        assert_eq!(utc_day(1_794_700_800_000), "2026-11-15");
    }

    #[test]
    fn a_day_is_checked() {
        assert!(check_day("2026-11-14").is_ok());
        for bad in ["2026-11-1", "2026/11/14", "", "2026-11-14T", "abcd-ef-gh"] {
            assert!(check_day(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn templates_splice_row_data_only_escaped() {
        let mount =
            Mount::new(crate::Layers::for_owner("b").unwrap()).action_base("/x/\"$h{urn:evil}/");
        for (_, _, text) in templates(&mount) {
            assert!(!text.contains("$r{{"), "{text}");
            assert!(!text.contains("$a{"), "{text}");
            // The base is the host's, escaped for the attribute and for compose: `$$` is a
            // literal `$`, so the marker in it is text.
            if text.contains("<form") {
                assert!(text.contains("action=\"/x/&quot;$$h{urn:evil}/"), "{text}");
            }
        }
    }

    /// A host serving these views under a strict CSP (`style-src 'self'`) refuses an inline
    /// style, so no template carries one; a host styles the `inquiry-*` classes instead.
    #[test]
    fn templates_carry_no_inline_style() {
        let mount = Mount::new(crate::Layers::for_owner("b").unwrap());
        for (_, name, text) in templates(&mount) {
            assert!(!text.contains("style="), "{name}: {text}");
            assert!(!text.contains("<style"), "{name}: {text}");
        }
        let all: String = templates(&mount).into_iter().map(|(_, _, t)| t).collect();
        assert!(all.contains("class=\"inquiry-message\""), "{all}");
    }
}
