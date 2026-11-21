//! The layer host on core 0.3, served to a core 0.4 kernel. TEST-ONLY, and temporary: this
//! crate runs on core 0.4.0, but `ikigai-layers` moves to it only in wave 5 of the core 0.4.0
//! cascade, so until then its space runs in a core 0.3 kernel of its own and every request for
//! a `urn:layer:` resource is carried across. Requests, capabilities and representation types
//! have one wire form in both lines, so they cross as JSON; an error is mapped variant by
//! variant. A carried answer crosses uncacheable: the core 0.3 kernel cuts its own threads,
//! and the core 0.4 kernel would never see them cut. Delete this module (and the
//! `ikigai-core-03` dev-dependency) when `ikigai-layers` is on core 0.4.0.

#![allow(dead_code)]

use std::sync::Arc;

use ikigai_core::{
    Bindings, Clock, Description, Endpoint, Error, Invocation, Representation, Request, Resolution,
    Resolved, Scope, Space, Time,
};
use ikigai_core_03 as core03;

/// A core 0.4 space answering for `layers`, a core 0.3 space, under `clock`.
pub fn space(layers: core03::EndpointSpace, clock: Arc<dyn Clock>) -> Bridge {
    let layers: Arc<dyn core03::Space> = Arc::new(layers);
    let kernel = core03::Kernel::new(layers.clone()).with_clock(Arc::new(Clock03(clock)));
    Bridge {
        layers,
        endpoint: Arc::new(Carry {
            kernel: Arc::new(kernel),
        }),
    }
}

pub struct Bridge {
    layers: Arc<dyn core03::Space>,
    endpoint: Arc<Carry>,
}

impl Space for Bridge {
    fn resolve(&self, request: &Request, _scope: &Scope) -> Resolution {
        match self
            .layers
            .resolve(&across(request), &core03::Scope::empty())
        {
            core03::Resolution::Miss => Resolution::Miss,
            core03::Resolution::Hit(_) => Resolution::Hit(Resolved::new(
                self.endpoint.clone() as Arc<dyn Endpoint>,
                Bindings::default(),
            )),
        }
    }
}

struct Carry {
    kernel: Arc<core03::Kernel>,
}

#[async_trait::async_trait]
impl Endpoint for Carry {
    async fn invoke(&self, inv: &Invocation<'_>) -> ikigai_core::Result<Representation> {
        let capability: core03::Capability = convert(inv.capability);
        let answer = self.kernel.issue(across(inv.request), &capability).await;
        match answer {
            Ok(rep) => Ok(Representation::new(convert(&rep.repr_type), rep.bytes)),
            Err(e) => Err(back(e)),
        }
    }

    fn name(&self) -> &str {
        "layers-on-core-0.3"
    }

    fn describe(&self) -> Description {
        Description::default()
    }
}

struct Clock03(Arc<dyn Clock>);

impl core03::Clock for Clock03 {
    fn now(&self) -> core03::Time {
        let Time(millis) = self.0.now();
        core03::Time(millis)
    }
}

/// A value of one line as the same value of the other, by its wire form.
fn convert<A: serde::Serialize, B: serde::de::DeserializeOwned>(value: &A) -> B {
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

fn across(request: &Request) -> core03::Request {
    convert(request)
}

fn back(e: core03::Error) -> Error {
    use core03::Error as E;
    match e {
        E::Unresolved(iri) => Error::Unresolved(convert(&iri)),
        E::MissingArgument(name) => Error::MissingArgument(name),
        E::InvalidArgument { name, detail } => Error::InvalidArgument { name, detail },
        E::Endpoint(s) => Error::Endpoint(s),
        E::Denied(s) => Error::Denied(s.into()),
        E::NotFound(s) => Error::NotFound(s),
        E::Gone(s) => Error::Gone(s),
        E::Conflict(s) => Error::Conflict(s),
        E::Timeout(s) => Error::Timeout(s),
        E::Unavailable(s) => Error::Unavailable(s),
        E::DepthExceeded { depth, target } => Error::DepthExceeded {
            depth,
            target: convert(&target),
        },
        E::RetryAfter { delay, detail } => Error::RetryAfter { delay, detail },
        E::OverBudget { detail } => Error::OverBudget { detail },
        E::Refused(s) => Error::Refused(s),
        E::Unsupported { verb, detail } => Error::Unsupported {
            verb: verb.map(|v| convert(&v)),
            detail,
        },
        E::GaveUp {
            attempts,
            elapsed,
            detail,
        } => Error::GaveUp {
            attempts,
            elapsed,
            detail,
        },
        // `Error` is non-exhaustive: a variant a later core 0.3 adds reads as its message.
        other => Error::Endpoint(other.to_string()),
    }
}
