//! A real layer host for the holding stage to work against: the holding, spam, inquiry and
//! intents tuplespaces and the examples channel of owner `b`, opened on the context and shapes
//! this crate ships, with a stand-in embedder and classifier, a clock the test moves, and the
//! stage and the layers client (embedding with the same embedder) mounted in front.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use futures::executor::block_on;
use ikigai_core::{
    ArgRef, Capability, Clock, EndpointSpace, Error, Exact, Fallback, FnEndpoint, Invocation, Iri,
    Kernel, ReprType, Representation, Request, Resolution, Scope, Space, Time, Verb,
};
use ikigai_inquiry::{example, holding_space, vocab, Layers, Mount, Policy, HOLDING_STAGE};
use serde_json::{json, Value};

pub const EDGE: &str = "urn:collab:participant:edge";
pub const BRIAN: &str = "urn:collab:participant:brian";
pub const WORKER_A: &str = "urn:collab:participant:worker-a";
pub const WORKER_B: &str = "urn:collab:participant:worker-b";
pub const INQ: &str = vocab::INQ;
pub const MODEL: &str = "embeddinggemma:latest";
pub const DIGEST: &str = "sha256:85462619ee72";
pub const CLASSIFIER: &str = "urn:iki:classify:inquiry-b:answer";

/// 2026-11-14T00:00:00Z.
pub const T0: u64 = 1_794_614_400_000;

/// A clock the test sets.
pub struct TestClock(AtomicU64);

impl Clock for TestClock {
    fn now(&self) -> Time {
        Time::from_millis(self.0.load(Ordering::SeqCst))
    }
}

/// What the stand-in embedder and classifier answer, and what they were asked.
#[derive(Default)]
pub struct Backends {
    /// The vector each text embeds to; a text not here embeds to `[1, 0, 0]`.
    pub vectors: Mutex<HashMap<String, Vec<f32>>>,
    pub embed_down: AtomicBool,
    /// Every `(model, text)` the embedder was asked.
    pub embedded: Mutex<Vec<(String, String)>>,
    /// The label the classifier answers; `None` is a failure (the server down).
    pub label: Mutex<Option<String>>,
    /// Every `(content, labels, context)` the classifier was asked.
    pub asked: Mutex<Vec<(String, String, Value)>>,
}

/// Holds every `rd` listing of the holding layer, once armed, until `n` callers have listed
/// (or a second has passed): so two workers both see a tuple before either takes it.
#[derive(Default)]
pub struct Gate {
    armed: AtomicBool,
    want: AtomicU64,
    seen: Mutex<u64>,
    all: Condvar,
}

impl Gate {
    pub fn arm(&self, n: u64) {
        self.want.store(n, Ordering::SeqCst);
        *self.seen.lock().unwrap() = 0;
        self.armed.store(true, Ordering::SeqCst);
    }

    fn pass(&self) {
        if !self.armed.load(Ordering::SeqCst) {
            return;
        }
        let want = self.want.load(Ordering::SeqCst);
        let mut seen = self.seen.lock().unwrap();
        *seen += 1;
        self.all.notify_all();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while *seen < want {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            seen = self.all.wait_timeout(seen, left).unwrap().0;
        }
    }
}

/// The layer space, with its holding listing passed through the [`Gate`].
struct Gated {
    inner: Arc<dyn Space>,
    gate: Arc<Gate>,
}

struct GatedEndpoint {
    inner: Arc<dyn ikigai_core::Endpoint>,
    gate: Arc<Gate>,
}

#[async_trait::async_trait]
impl ikigai_core::Endpoint for GatedEndpoint {
    async fn invoke(&self, inv: &Invocation<'_>) -> ikigai_core::Result<Representation> {
        let answer = self.inner.invoke(inv).await;
        let args = &inv.request.args;
        if inv.request.verb == Verb::Source
            && args.get("tuple").is_none()
            && args.get("state").is_none()
        {
            self.gate.pass();
        }
        answer
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn describe(&self) -> ikigai_core::Description {
        self.inner.describe()
    }
}

impl Space for Gated {
    fn resolve(&self, request: &Request, scope: &Scope) -> Resolution {
        let resolution = self.inner.resolve(request, scope);
        if request.target.as_str() != "urn:layer:inquiry-hold-b:tuples" {
            return resolution;
        }
        let gate = self.gate.clone();
        resolution.map_endpoint(move |inner| {
            Arc::new(GatedEndpoint { inner, gate }) as Arc<dyn ikigai_core::Endpoint>
        })
    }
}

pub struct Fixture {
    dir: PathBuf,
    pub kernel: Kernel,
    pub clock: Arc<TestClock>,
    pub backends: Arc<Backends>,
    pub gate: Arc<Gate>,
    _host: Arc<ikigai_layers::Host>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    pub fn set_time(&self, millis: u64) {
        self.clock.0.store(millis, Ordering::SeqCst);
    }

    pub fn embed_to(&self, text: &str, vector: &[f32]) {
        self.backends
            .vectors
            .lock()
            .unwrap()
            .insert(text.into(), vector.to_vec());
    }

    pub fn classifier_says(&self, label: Option<&str>) {
        *self.backends.label.lock().unwrap() = label.map(str::to_string);
    }
}

/// A worker's grant: everything one pass needs, and nothing else.
pub fn worker(principal: &str) -> Capability {
    Capability::scoped([
        "urn:cap:layer:read:inquiry-hold-b",
        "urn:cap:layer:take:inquiry-hold-b",
        "urn:cap:layer:read:inquiry-spam-b",
        "urn:cap:layer:out:inquiry-spam-b",
        "urn:cap:layer:read:inquiry-b",
        "urn:cap:layer:out:inquiry-b",
        "urn:cap:layer:read:inquiry-example-b",
    ])
    .with_principal(principal)
    .unwrap()
}

pub fn edge() -> Capability {
    Capability::scoped([
        "urn:cap:layer:out:inquiry-hold-b",
        "urn:cap:layer:out:inquiry-spam-b",
    ])
    .with_principal(EDGE)
    .unwrap()
}

/// A person: confirms examples, directly or through the client's `confirm-spam`.
pub fn brian() -> Capability {
    Capability::scoped([
        "urn:cap:layer:say:inquiry-example-b",
        "urn:cap:layer:read:inquiry-spam-b",
        "urn:cap:layer:take:inquiry-spam-b",
        "urn:cap:layer:out:inquiry-intent-b",
    ])
    .with_principal(BRIAN)
    .unwrap()
}

/// Reads everything, for the tests' own checks.
pub fn auditor() -> Capability {
    Capability::scoped(
        [
            "inquiry-b",
            "inquiry-spam-b",
            "inquiry-hold-b",
            "inquiry-example-b",
        ]
        .map(|l| format!("urn:cap:layer:read:{l}")),
    )
    .with_principal("urn:collab:participant:auditor")
    .unwrap()
}

fn write(dir: &std::path::Path, path: &str, text: &str) {
    std::fs::write(dir.join(path), text).unwrap();
}

pub fn fixture(name: &str, policy: Policy) -> Fixture {
    let dir = std::env::temp_dir().join(format!(
        "ikigai-inquiry-holding-{name}-{}",
        std::process::id()
    ));
    let (config, state) = (dir.join("config"), dir.join("state"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(config.join("layers")).unwrap();
    std::fs::create_dir_all(config.join("collabs")).unwrap();
    write(&config, "layers/inquiry.context.jsonld", vocab::CONTEXT);
    write(&config, "layers/inquiry.shapes.ttl", vocab::INQUIRY_SHAPES);
    write(&config, "layers/example.shapes.ttl", vocab::EXAMPLE_SHAPES);
    write(&config, "layers/intent.shapes.ttl", vocab::INTENT_SHAPES);
    write(
        &config,
        "layers/inquiry-intent-b.toml",
        "model = \"tuplespace\"\nshapes = \"intent.shapes.ttl\"\n\
         context = \"inquiry.context.jsonld\"\n",
    );
    for layer in ["inquiry-b", "inquiry-spam-b", "inquiry-hold-b"] {
        write(
            &config,
            &format!("layers/{layer}.toml"),
            "model = \"tuplespace\"\nshapes = \"inquiry.shapes.ttl\"\n\
             context = \"inquiry.context.jsonld\"\n",
        );
    }
    write(
        &config,
        "layers/inquiry-example-b.toml",
        "model = \"channel\"\nshapes = \"example.shapes.ttl\"\n\
         context = \"inquiry.context.jsonld\"\n",
    );
    write(
        &config,
        "participants.ttl",
        "<urn:collab:participant:brian> a <http://xmlns.com/foaf/0.1/Person> .\n\
         <urn:collab:participant:auditor> a <http://xmlns.com/foaf/0.1/Person> .\n\
         <urn:collab:participant:edge> a <http://www.w3.org/ns/prov#SoftwareAgent> ; \
         <http://www.w3.org/ns/prov#actedOnBehalfOf> <urn:collab:participant:brian> .\n\
         <urn:collab:participant:worker-a> a <http://www.w3.org/ns/prov#SoftwareAgent> ; \
         <http://www.w3.org/ns/prov#actedOnBehalfOf> <urn:collab:participant:brian> .\n\
         <urn:collab:participant:worker-b> a <http://www.w3.org/ns/prov#SoftwareAgent> ; \
         <http://www.w3.org/ns/prov#actedOnBehalfOf> <urn:collab:participant:brian> .\n",
    );
    let manifest = "\
        [[mount]]\nlayer = \"inquiry-hold-b\"\nedge = [\"out\"]\nworker-a = [\"read\", \"take\"]\n\
        worker-b = [\"read\", \"take\"]\nauditor = [\"read\"]\n\n\
        [[mount]]\nlayer = \"inquiry-spam-b\"\nedge = [\"out\"]\nbrian = [\"read\", \"take\"]\n\
        worker-a = [\"read\", \"out\"]\n\
        worker-b = [\"read\", \"out\"]\nauditor = [\"read\"]\n\n\
        [[mount]]\nlayer = \"inquiry-b\"\nworker-a = [\"read\", \"out\"]\n\
        worker-b = [\"read\", \"out\"]\nauditor = [\"read\"]\n\n\
        [[mount]]\nlayer = \"inquiry-example-b\"\nbrian = [\"say\"]\nworker-a = [\"read\"]\n\
        worker-b = [\"read\"]\nauditor = [\"read\"]\n\n\
        [[mount]]\nlayer = \"inquiry-intent-b\"\nbrian = [\"out\"]\nauditor = [\"read\"]\n";
    write(&config, "collabs/b.toml", manifest);
    write(
        &config,
        "state.key",
        age::secrecy::ExposeSecret::expose_secret(&age::x25519::Identity::generate().to_string()),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            config.join("state.key"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let key = ikigai_layers::StateKey::load(config.join("state.key")).unwrap();
    let host = ikigai_layers::Host::open(&config, &state, &key).expect("the shipped shapes open");

    let backends = Arc::new(Backends::default());
    let b = backends.clone();
    let embedder = FnEndpoint::new("test-embed", move |inv| {
        let model = inv.inline_str("model")?.to_string();
        let text = inv.inline_str("text")?.to_string();
        b.embedded.lock().unwrap().push((model, text.clone()));
        if b.embed_down.load(Ordering::SeqCst) {
            return Err(Error::Endpoint("connection refused".into()));
        }
        let vector = b
            .vectors
            .lock()
            .unwrap()
            .get(&text)
            .cloned()
            .unwrap_or_else(|| vec![1.0, 0.0, 0.0]);
        let body = json!({
            "vector": vector, "model": MODEL, "digest": DIGEST, "dimensions": vector.len(),
        });
        Ok(Representation::new(
            ReprType::new("application/json"),
            body.to_string().into_bytes(),
        ))
    });
    let b = backends.clone();
    let classifier = FnEndpoint::new("test-classify", move |inv| {
        let content = inv.inline_str("content")?.to_string();
        let labels = inv.inline_str("labels")?.to_string();
        let context: Value = serde_json::from_str(inv.inline_str("context")?).unwrap();
        b.asked.lock().unwrap().push((content, labels, context));
        match b.label.lock().unwrap().clone() {
            None => Err(Error::Endpoint("the model server is down".into())),
            Some(label) => Ok(Representation::new(
                ReprType::new("application/json"),
                json!({ "label": label }).to_string().into_bytes(),
            )),
        }
    });
    let backend_space = EndpointSpace::new()
        .bind(Exact::new("urn:llm:embed"), embedder)
        .bind(Exact::new(CLASSIFIER), classifier);
    let gate = Arc::new(Gate::default());
    let layers = Gated {
        inner: Arc::new(ikigai_layers::space(&host)),
        gate: gate.clone(),
    };
    let client = ikigai_inquiry::space(
        Mount::new(Layers::for_owner("b").unwrap())
            .embedder(Iri::parse("urn:llm:embed").unwrap())
            .embed_model(policy.embed_model.clone()),
    );
    let spaces: Vec<Arc<dyn Space>> = vec![
        Arc::new(holding_space(policy)),
        Arc::new(client),
        Arc::new(backend_space),
        Arc::new(layers),
    ];
    let clock = Arc::new(TestClock(AtomicU64::new(T0)));
    let kernel = Kernel::new(Arc::new(Fallback::new(spaces))).with_clock(clock.clone());
    Fixture {
        dir,
        kernel,
        clock,
        backends,
        gate,
        _host: host,
    }
}

pub fn issue(
    kernel: &Kernel,
    cap: &Capability,
    verb: Verb,
    iri: &str,
    args: &[(&str, &str)],
) -> Result<Representation, Error> {
    let mut request = Request::new(verb, Iri::parse(iri).unwrap());
    for (name, value) in args {
        request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    block_on(kernel.issue(request, cap))
}

pub fn text(rep: Representation) -> String {
    String::from_utf8(rep.bytes).unwrap()
}

/// One pass of the stage, as `worker`.
pub fn run(f: &Fixture, worker_principal: &str, min_age: u64) -> Result<String, Error> {
    issue(
        &f.kernel,
        &worker(worker_principal),
        Verb::Sink,
        HOLDING_STAGE,
        &[("min-age", &min_age.to_string())],
    )
    .map(text)
}

/// What the edge drops into holding: one inquiry.
pub fn arrive(f: &Fixture, id: &str, words: &str) {
    let node = json!({
        "@id": format!("urn:layer:inquiry-hold-b:inquiry:id:{id}"),
        "@type": format!("{INQ}Inquiry"),
        format!("{INQ}inquiry"): id,
        format!("{INQ}site"): "a.example",
        format!("{INQ}text"): words,
        format!("{INQ}sender"): sender_of(id),
        format!("{INQ}replyTo"): "visitor@a.example",
        format!("{INQ}reason"): "score",
        format!("{INQ}received"): {
            "@value": "2026-11-13T23:59:00.000Z",
            "@type": "http://www.w3.org/2001/XMLSchema#dateTime"
        },
    });
    issue(
        &f.kernel,
        &edge(),
        Verb::Sink,
        "urn:layer:inquiry-hold-b:tuples",
        &[("content", &node.to_string())],
    )
    .expect("the shapes admit what the edge drops");
}

/// What the edge drops into spam: one inquiry, for a person to confirm.
pub fn arrive_spam(f: &Fixture, id: &str, words: &str) {
    let node = json!({
        "@id": format!("urn:layer:inquiry-spam-b:inquiry:id:{id}"),
        "@type": format!("{INQ}Inquiry"),
        format!("{INQ}inquiry"): id,
        format!("{INQ}site"): "a.example",
        format!("{INQ}text"): words,
        format!("{INQ}reason"): "score",
    });
    issue(
        &f.kernel,
        &edge(),
        Verb::Sink,
        "urn:layer:inquiry-spam-b:tuples",
        &[("content", &node.to_string())],
    )
    .expect("the shapes admit what the edge drops");
}

/// A person confirms spam through the layers client's `confirm-spam`.
pub fn confirm_through_client(f: &Fixture, inquiry: &str) -> Result<String, Error> {
    issue(
        &f.kernel,
        &brian(),
        Verb::Sink,
        "urn:iki:inquiry:action:name:confirm-spam",
        &[("inquiry", inquiry)],
    )
    .map(text)
}

/// A person confirms spam: one example in the channel, embedded by `model` at `digest`.
pub fn confirm(f: &Fixture, inquiry: &str, words: &str, vector: &[f32], model: &str, digest: &str) {
    let node = json!({
        "@id": "urn:layer:new",
        "@type": format!("{INQ}SpamExample"),
        format!("{INQ}inquiry"): inquiry,
        format!("{INQ}site"): "a.example",
        format!("{INQ}text"): words,
        format!("{INQ}embedding"): example::encode(vector),
        format!("{INQ}embeddingModel"): model,
        format!("{INQ}embeddingDigest"): digest,
        format!("{INQ}label"): "seo",
    });
    issue(
        &f.kernel,
        &brian(),
        Verb::Sink,
        "urn:layer:inquiry-example-b:say",
        &[("content", &node.to_string())],
    )
    .expect("the shapes admit a confirmed example");
}

/// The pending tuples of `layer`, as the layer's JSON-LD graph.
pub fn pending(f: &Fixture, layer: &str) -> Vec<Value> {
    let rep = issue(
        &f.kernel,
        &auditor(),
        Verb::Source,
        &format!("urn:layer:{layer}:tuples"),
        &[("as", "application/ld+json")],
    )
    .unwrap();
    let doc: Value = serde_json::from_slice(&rep.bytes).unwrap();
    doc["@graph"].as_array().cloned().unwrap_or_default()
}

/// The receipts of `layer`, as its JSON list.
pub fn receipts(f: &Fixture, layer: &str) -> Vec<Value> {
    let rep = issue(
        &f.kernel,
        &auditor(),
        Verb::Source,
        &format!("urn:layer:{layer}:tuples"),
        &[("state", "taken"), ("as", "application/json")],
    )
    .unwrap();
    let doc: Value = serde_json::from_slice(&rep.bytes).unwrap();
    doc["receipts"].as_array().cloned().unwrap_or_default()
}

/// The IRIs of the examples channel's entries.
pub fn example_ids(f: &Fixture) -> Vec<String> {
    let rep = issue(
        &f.kernel,
        &auditor(),
        Verb::Source,
        "urn:layer:inquiry-example-b:entries",
        &[("as", "application/ld+json")],
    )
    .unwrap();
    let doc: Value = serde_json::from_slice(&rep.bytes).unwrap();
    doc["@graph"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| n["@id"].as_str().map(str::to_string))
        .collect()
}

/// A compacted node's value for `term`, as a string.
pub fn field(node: &Value, term: &str) -> Option<String> {
    match node.get(term)? {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Object(o) => o.get("@value").map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }),
        _ => None,
    }
}

/// A sender key in the form the edge writes (64 lowercase hex digits), standing in for the
/// keyed hash of inquiry `id`'s sender.
pub fn sender_of(id: &str) -> String {
    blake3::hash(format!("sender-of-{id}").as_bytes())
        .to_hex()
        .to_string()
}
