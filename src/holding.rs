//! The holding stage: `urn:iki:inquiry:stage:name:holding`, a Sink. One write is one pass:
//!
//! 1. **Finish what this worker lost** (its first pass in this process, and the pass after any
//!    pass that could not send a tuple on): re-read the holding layer's receipts, and for each
//!    take of THIS worker whose inquiry is in neither the spam nor the inquiry layer, read the
//!    tuple back from its receipt, judge it and send it on.
//! 2. **List** the holding tuplespace (its JSON listing gives each tuple's `created` and
//!    subject), and leave every tuple younger than `min-age=` for a later pass.
//! 3. **Take** each older tuple BY ITS SUBJECT. A second taker is answered `Conflict` naming
//!    the first: this worker moves on.
//! 4. **Judge** it ([`crate::funnel`]) and **out** the result into the spam or the inquiry
//!    layer, as `urn:layer:{layer}:inquiry:id:{id}`. Take first, then out: a tuple is never in
//!    two places, and one whose out was lost is in this worker's receipts, which step 1 reads.
//!
//! The out's subject is the inquiry's id under the destination layer, so finishing a lost out
//! twice is one tuple: a layer that already holds that subject (pending, or taken since) has
//! it, and a replayed out the layer refuses as a used subject is the out that was lost.
//!
//! In **shadow** ([`Policy::shadow`]) every inquiry goes to the inquiry layer, with what the
//! funnel would have done recorded on it (`inq:verdict`, and the example and score when one
//! decided).
//!
//! The stage holds no authority: every take, read and out runs under the caller's grant. A
//! worker holds `read` and `take` on holding, `read` and `out` on spam and inquiry, `read` on
//! the examples channel, and whatever the embedder and the classifier require.
//!
//! The answer is `text/plain`, one line per tuple the pass handled and a closing count. It
//! names subjects, layers and reasons, never an inquiry's text.

use std::collections::HashSet;
use std::sync::Mutex;

use async_trait::async_trait;
use ikigai_core::{
    ArgSpec, Description, Endpoint, Error, Invocation, ReprType, Representation, Result, Verb,
};
use serde_json::{json, Map, Value};

use crate::example;
use crate::funnel::{self, Judgment, Reason, Verdict};
use crate::layer::{self, inquiry_subject, Held};
use crate::policy::Policy;
use crate::vocab::{inq, XSD};

/// The stage's IRI.
pub const HOLDING_STAGE: &str = "urn:iki:inquiry:stage:name:holding";

/// The holding stage of one owner's layers.
pub struct HoldingStage {
    policy: Policy,
    /// The workers whose receipts this process has finished: a worker not in it (one that has
    /// not run here yet, or whose last pass lost an out) re-reads its receipts first.
    finished: Mutex<HashSet<String>>,
}

impl HoldingStage {
    /// The stage for `policy`.
    pub fn new(policy: Policy) -> HoldingStage {
        HoldingStage {
            policy,
            finished: Mutex::new(HashSet::new()),
        }
    }

    /// Its policy.
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    fn finished(&self, worker: &str) -> bool {
        self.finished.lock().expect("lock").contains(worker)
    }

    fn mark(&self, worker: &str, finished: bool) {
        let mut set = self.finished.lock().expect("lock");
        if finished {
            set.insert(worker.to_string());
        } else {
            set.remove(worker);
        }
    }
}

/// `min-age=`: whole seconds.
fn min_age(inv: &Invocation<'_>) -> Result<u64> {
    let text = inv.inline_str("min-age")?;
    text.trim().parse().map_err(|_| Error::InvalidArgument {
        name: "min-age".into(),
        detail: format!(
            "`{text}` is not a whole number of seconds: a tuple younger than min-age is left \
             for a later pass"
        ),
    })
}

/// What one pass did, as the answer says it.
#[derive(Default)]
struct Pass {
    lines: Vec<String>,
    judged: usize,
    recovered: usize,
    young: usize,
    lost_race: usize,
    unsent: usize,
}

impl Pass {
    fn answer(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            out.push_str(line);
            out.push('\n');
        }
        let n = |count: usize, one: &str, many: &str| {
            format!("{count} {}", if count == 1 { one } else { many })
        };
        out.push_str(&format!(
            "{} sent on; {} finished from this worker's receipts; {} left for a later pass, \
             younger than min-age; {}; {}.\n",
            n(self.judged, "inquiry", "inquiries"),
            n(self.recovered, "inquiry", "inquiries"),
            n(self.young, "inquiry", "inquiries"),
            n(
                self.lost_race,
                "taken first by another worker",
                "taken first by other workers"
            ),
            n(
                self.unsent,
                "taken but not yet sent on",
                "taken but not yet sent on"
            ),
        ));
        out
    }
}

/// How a judgment is written on the tuple sent on, and where it goes.
fn sent(policy: &Policy, judgment: &Judgment) -> (String, Map<String, Value>) {
    let mut extra = Map::new();
    let reason = if policy.shadow {
        "shadow"
    } else {
        judgment.reason.word()
    };
    extra.insert(inq("reason"), json!(reason));
    extra.insert(inq("verdict"), json!(judgment.verdict.word()));
    match &judgment.reason {
        Reason::Example { id, score } => {
            extra.insert(inq("example"), json!(id));
            extra.insert(
                inq("score"),
                json!({ "@value": format!("{score:.6}"), "@type": format!("{XSD}double") }),
            );
        }
        Reason::Unjudged { .. } => {
            extra.insert(
                inq("unjudged"),
                json!({ "@value": "true", "@type": format!("{XSD}boolean") }),
            );
        }
        Reason::Model | Reason::Abstained => {}
    }
    let layer = match (policy.shadow, judgment.verdict) {
        (false, Verdict::Spam) => policy.spam.clone(),
        _ => policy.inquiry.clone(),
    };
    (layer, extra)
}

/// The words a line gives a judgment.
fn because(judgment: &Judgment, shadow: bool) -> String {
    let why = match &judgment.reason {
        Reason::Example { id, score } => format!("confirmed example <{id}>, score {score:.3}"),
        Reason::Model => "the local model's answer".into(),
        Reason::Abstained => "the local model gave no answer of spam or not-spam".into(),
        Reason::Unjudged { stage } => {
            let what = match *stage {
                "embed" => "the embedder",
                "examples" => "the examples channel",
                _ => "the local model",
            };
            format!("unjudged, {what} failed")
        }
    };
    if shadow {
        format!(
            "shadow; the funnel would send it to {}: {why}",
            judgment.verdict.word()
        )
    } else {
        why
    }
}

impl HoldingStage {
    /// Judge `inquiry` and send it on; answers the line that says so. Fails only when the out
    /// fails, which leaves it in this worker's receipts.
    async fn send_on(
        &self,
        inv: &Invocation<'_>,
        inquiry: &Held,
        examples: &Result<Vec<example::SpamExample>>,
    ) -> Result<String> {
        let judgment = funnel::judge(inv, &self.policy, &inquiry.text, examples).await;
        let (layer, extra) = sent(&self.policy, &judgment);
        let tuple = inquiry.tuple(&layer, extra);
        match layer::out(inv, &layer, &tuple).await {
            // A used subject is this inquiry already sent on: a replay of an out whose reply
            // was lost, judged again.
            Ok(()) | Err(Error::Conflict(_)) => Ok(format!(
                "sent to {layer} ({})",
                because(&judgment, self.policy.shadow)
            )),
            Err(e) => Err(e),
        }
    }

    /// Step 1: every take of `worker` whose inquiry reached neither layer, finished.
    async fn recover(
        &self,
        inv: &Invocation<'_>,
        worker: &str,
        examples: &Result<Vec<example::SpamExample>>,
        pass: &mut Pass,
    ) -> Result<bool> {
        let p = &self.policy;
        let mut all_finished = true;
        for taken in layer::receipts(inv, &p.holding).await? {
            if taken.taker != worker {
                continue;
            }
            let graph = layer::receipt(inv, &taken.receipt).await?;
            let Some(inquiry) = Held::from_graph(&graph) else {
                pass.lines.push(format!(
                    "{}: its receipt <{}> holds no inquiry this stage can read, so it is not \
                     sent on",
                    taken.subject, taken.receipt
                ));
                continue;
            };
            let mut there = false;
            for to in [&p.spam, &p.inquiry] {
                if layer::holds(inv, to, &inquiry_subject(to, &inquiry.id)).await? {
                    there = true;
                    break;
                }
            }
            if there {
                continue;
            }
            match self.send_on(inv, &inquiry, examples).await {
                Ok(line) => {
                    pass.recovered += 1;
                    pass.lines.push(format!(
                        "{}: finished from receipt <{}>, {line}",
                        taken.subject, taken.receipt
                    ));
                }
                Err(e) => {
                    all_finished = false;
                    pass.unsent += 1;
                    pass.lines.push(format!(
                        "{}: still not sent on ({e}); the next pass tries again",
                        taken.subject
                    ));
                }
            }
        }
        Ok(all_finished)
    }

    async fn pass(&self, inv: &Invocation<'_>) -> Result<Representation> {
        let min_age = min_age(inv)?;
        let worker = inv
            .capability
            .principal()
            .ok_or_else(|| {
                Error::Denied(
                    "the holding stage takes as a worker, and this request names none: its \
                     receipts are how a lost out is finished"
                        .into(),
                )
            })?
            .to_string();
        let now = inv
            .now()
            .ok_or_else(|| {
                Error::Endpoint(
                    "this host gives the kernel no clock, and the holding stage needs one to \
                     tell a tuple's age"
                        .into(),
                )
            })?
            .as_millis();
        let p = &self.policy;
        // Read once per pass, into memory: the channel is the store.
        let examples = example::load(inv, &p.examples).await;
        let mut pass = Pass::default();
        if !self.finished(&worker) {
            let done = self.recover(inv, &worker, &examples, &mut pass).await?;
            self.mark(&worker, done);
        }
        let min_ms = min_age.saturating_mul(1000);
        for listed in layer::listed(inv, &p.holding).await? {
            let old_enough = crate::date::millis(&listed.created)
                .is_some_and(|created| now.saturating_sub(created) >= min_ms);
            if !old_enough {
                pass.young += 1;
                continue;
            }
            let graph = match layer::take(inv, &p.holding, &listed.subject).await {
                Ok(graph) => graph,
                Err(Error::Conflict(why)) => {
                    pass.lost_race += 1;
                    pass.lines
                        .push(format!("{}: not taken, {why}", listed.subject));
                    continue;
                }
                // Taken and gone between the listing and the take.
                Err(Error::NotFound(_)) => {
                    pass.lost_race += 1;
                    continue;
                }
                Err(e) => return Err(e),
            };
            let Some(inquiry) = Held::from_graph(&graph) else {
                pass.unsent += 1;
                pass.lines.push(format!(
                    "{}: taken, but it holds no inquiry this stage can read, so it is not sent \
                     on; it stays in this worker's receipts",
                    listed.subject
                ));
                continue;
            };
            match self.send_on(inv, &inquiry, &examples).await {
                Ok(line) => {
                    pass.judged += 1;
                    pass.lines.push(format!("{}: {line}", listed.subject));
                }
                Err(e) => {
                    self.mark(&worker, false);
                    pass.unsent += 1;
                    pass.lines.push(format!(
                        "{}: taken, but not sent on yet ({e}); this worker's next pass finishes \
                         it from its receipt",
                        listed.subject
                    ));
                }
            }
        }
        Ok(Representation::new(
            ReprType::new("text/plain").with_param("charset", "utf-8"),
            pass.answer().into_bytes(),
        ))
    }
}

#[async_trait]
impl Endpoint for HoldingStage {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        if inv.request.verb != Verb::Sink {
            return Err(Error::InvalidArgument {
                name: "verb".into(),
                detail: format!("{HOLDING_STAGE} is a Sink: write to it to run one pass"),
            });
        }
        self.pass(inv).await
    }

    fn name(&self) -> &str {
        "inquiry-holding-stage"
    }

    fn describe(&self) -> Description {
        Description::new(self.name())
            .summary(
                "One pass of the holding stage: take each holding inquiry at least min-age \
                 old, judge it against the confirmed spam examples and then a local model, and \
                 send it to the spam or the inquiry layer",
            )
            .verb(Verb::Sink)
            .input(
                ArgSpec::new("min-age")
                    .summary(
                        "the youngest a tuple may be to be taken, in whole seconds (the host's \
                         policy row)",
                    )
                    .class(format!("{XSD}nonNegativeInteger")),
            )
            .output("text/plain")
    }
}
