//! The day counters: aggregates only. Per site, how many inquiries went to each layer and how
//! many a block dropped; and the confusion of score class × the label a person gave. A counter
//! is a number under a date: nothing names a sender, an inquiry or a person.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};

use crate::store::Store;

/// The most sites one day counts. Past it a new site's inquiry is still routed, and counted
/// under [`OTHER_SITE`].
pub const MAX_SITES_PER_DAY: usize = 256;

/// The site an inquiry is counted under once a day already counts [`MAX_SITES_PER_DAY`].
pub const OTHER_SITE: &str = "other";

/// One day's counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Day {
    /// Site → layer → inquiries routed there.
    pub routed: BTreeMap<String, BTreeMap<String, u64>>,
    /// Site → inquiries a block dropped.
    pub blocked: BTreeMap<String, u64>,
    /// Score class → a person's label → how many.
    pub confusion: BTreeMap<String, BTreeMap<String, u64>>,
}

impl Day {
    fn site<'a>(&self, site: &'a str) -> &'a str {
        let known = self.routed.contains_key(site) || self.blocked.contains_key(site);
        let sites: std::collections::BTreeSet<&String> =
            self.routed.keys().chain(self.blocked.keys()).collect();
        if known || sites.len() < MAX_SITES_PER_DAY {
            site
        } else {
            OTHER_SITE
        }
    }

    /// The sites face, keys sorted.
    pub(crate) fn sites_json(&self, date: &str) -> Value {
        let mut sites = Map::new();
        let names: std::collections::BTreeSet<&String> =
            self.routed.keys().chain(self.blocked.keys()).collect();
        for site in names {
            sites.insert(
                site.clone(),
                json!({
                    "blocked": self.blocked.get(site).copied().unwrap_or(0),
                    "routed": self.routed.get(site).cloned().unwrap_or_default(),
                }),
            );
        }
        json!({ "date": date, "sites": sites })
    }

    /// The sites face as text: `date <date>`, then per site one line
    /// `site <site> blocked <n>` and one `site <site> layer <layer> <n>` per layer.
    pub(crate) fn sites_text(&self, date: &str) -> String {
        let mut out = format!("date {date}\n");
        let names: std::collections::BTreeSet<&String> =
            self.routed.keys().chain(self.blocked.keys()).collect();
        for site in names {
            let blocked = self.blocked.get(site).copied().unwrap_or(0);
            out.push_str(&format!("site {site} blocked {blocked}\n"));
            for (layer, n) in self.routed.get(site).into_iter().flatten() {
                out.push_str(&format!("site {site} layer {layer} {n}\n"));
            }
        }
        out
    }

    pub(crate) fn confusion_json(&self, date: &str) -> Value {
        json!({ "confusion": self.confusion, "date": date })
    }

    /// The confusion face as text: `date <date>`, then one `<class> <label> <n>` line each.
    pub(crate) fn confusion_text(&self, date: &str) -> String {
        let mut out = format!("date {date}\n");
        for (class, labels) in &self.confusion {
            for (label, n) in labels {
                out.push_str(&format!("{class} {label} {n}\n"));
            }
        }
        out
    }

    fn to_json(&self) -> Value {
        json!({
            "blocked": self.blocked,
            "confusion": self.confusion,
            "routed": self.routed,
        })
    }

    fn from_json(value: &Value) -> Option<Day> {
        fn counts(value: &Value) -> Option<BTreeMap<String, u64>> {
            value
                .as_object()?
                .iter()
                .map(|(k, v)| Some((k.clone(), v.as_u64()?)))
                .collect()
        }
        fn nested(value: &Value) -> Option<BTreeMap<String, BTreeMap<String, u64>>> {
            value
                .as_object()?
                .iter()
                .map(|(k, v)| Some((k.clone(), counts(v)?)))
                .collect()
        }
        Some(Day {
            routed: nested(value.get("routed")?)?,
            blocked: counts(value.get("blocked")?)?,
            confusion: nested(value.get("confusion")?)?,
        })
    }
}

/// The counters, one document per day.
pub struct Counts {
    store: Arc<dyn Store>,
    days: Mutex<BTreeMap<String, Day>>,
}

impl std::fmt::Debug for Counts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Counts(..)")
    }
}

/// One count to add.
pub(crate) enum Tally<'a> {
    Routed { site: &'a str, layer: &'a str },
    Blocked { site: &'a str },
    Labeled { class: &'a str, label: &'a str },
}

impl Counts {
    /// The counters in `store`.
    pub fn new(store: Arc<dyn Store>) -> Counts {
        Counts {
            store,
            days: Mutex::new(BTreeMap::new()),
        }
    }

    fn document(date: &str) -> String {
        format!("count-date-{date}")
    }

    fn load<'a>(
        &self,
        days: &'a mut BTreeMap<String, Day>,
        date: &str,
    ) -> Result<&'a mut Day, String> {
        if !days.contains_key(date) {
            let day = match self.store.load(&Self::document(date))? {
                None => Day::default(),
                Some(bytes) => serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .as_ref()
                    .and_then(Day::from_json)
                    .ok_or_else(|| format!("the counts of {date} are not counts: refused"))?,
            };
            // Only the days in use stay in memory: today's, and whichever one was just read.
            if days.len() >= 4 {
                days.clear();
            }
            days.insert(date.to_string(), day);
        }
        Ok(days.get_mut(date).expect("just loaded"))
    }

    /// The counts of `date`.
    pub fn day(&self, date: &str) -> Result<Day, String> {
        let mut days = self.days.lock().expect("count lock");
        self.load(&mut days, date).cloned()
    }

    /// Add one to a count of `date`, saved before the answer.
    pub(crate) fn add(&self, date: &str, tally: Tally<'_>) -> Result<(), String> {
        let mut days = self.days.lock().expect("count lock");
        let day = self.load(&mut days, date)?;
        let mut next = day.clone();
        match tally {
            Tally::Routed { site, layer } => {
                let site = next.site(site).to_string();
                *next
                    .routed
                    .entry(site)
                    .or_default()
                    .entry(layer.to_string())
                    .or_default() += 1;
            }
            Tally::Blocked { site } => {
                let site = next.site(site).to_string();
                *next.blocked.entry(site).or_default() += 1;
            }
            Tally::Labeled { class, label } => {
                *next
                    .confusion
                    .entry(class.to_string())
                    .or_default()
                    .entry(label.to_string())
                    .or_default() += 1;
            }
        }
        self.store
            .save(&Self::document(date), next.to_json().to_string().as_bytes())?;
        *day = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;

    #[test]
    fn counts_add_up_and_survive_a_reopen() {
        let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let counts = Counts::new(store.clone());
        let d = "2026-11-14";
        counts
            .add(
                d,
                Tally::Routed {
                    site: "a",
                    layer: "inquiry",
                },
            )
            .unwrap();
        counts
            .add(
                d,
                Tally::Routed {
                    site: "a",
                    layer: "inquiry",
                },
            )
            .unwrap();
        counts.add(d, Tally::Blocked { site: "b" }).unwrap();
        counts
            .add(
                d,
                Tally::Labeled {
                    class: "unsure",
                    label: "spam",
                },
            )
            .unwrap();
        let day = Counts::new(store).day(d).unwrap();
        assert_eq!(day.routed["a"]["inquiry"], 2);
        assert_eq!(day.blocked["b"], 1);
        assert_eq!(day.confusion["unsure"]["spam"], 1);
    }

    #[test]
    fn past_the_site_bound_a_new_site_is_counted_as_other() {
        let counts = Counts::new(Arc::new(MemoryStore::new()));
        let d = "2026-11-14";
        for i in 0..MAX_SITES_PER_DAY {
            let site = format!("s{i}");
            counts.add(d, Tally::Blocked { site: &site }).unwrap();
        }
        counts.add(d, Tally::Blocked { site: "late" }).unwrap();
        counts.add(d, Tally::Blocked { site: "s0" }).unwrap();
        let day = counts.day(d).unwrap();
        assert_eq!(day.blocked[OTHER_SITE], 1);
        assert_eq!(day.blocked["s0"], 2);
        assert!(!day.blocked.contains_key("late"));
    }
}
