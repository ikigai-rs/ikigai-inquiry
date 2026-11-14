//! `ikigai-inquiry`: inquiries from public contact forms, as resources.
//!
//! Scaffold. The module will bind, under `urn:iki:inquiry:*`:
//!
//! - the edge half: a router that scores a submitted inquiry and puts it into one layer
//!   (holding, spam or inquiry), a block set keyed by a hash of the sender, and aggregate
//!   day counters;
//! - a holding stage that takes each borderline inquiry once, compares it with confirmed spam
//!   examples, asks a local model only when no example is close, and sends it on;
//! - a layers client: views and actions (move, confirm, block, respond) that any host mounts.
//!
//! Nothing here names a site, an owner or a host. The design is ledger #3680.

#![forbid(unsafe_code)]
