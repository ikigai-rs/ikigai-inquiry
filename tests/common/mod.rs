//! A real layer host for the client to work against: the inquiry, spam, holding and intents
//! tuplespaces and the examples channel of owner `b`, opened on the context and shapes this
//! crate ships, with the client mounted in front of it.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use futures::executor::block_on;
use ikigai_core::{
    ArgRef, Capability, EndpointSpace, Error, Exact, Fallback, FnEndpoint, Iri, Kernel, ReprType,
    Representation, Request, Space, SystemClock, Verb,
};
use ikigai_inquiry::{vocab, Layers, Mount};
use serde_json::{json, Value};

pub const BRIAN: &str = "urn:collab:participant:brian";
pub const CHRIS: &str = "urn:collab:participant:chris";
pub const EDGE: &str = "urn:collab:participant:edge";
pub const INQ: &str = vocab::INQ;
pub const EMBEDDER: &str = "urn:test:embed";
pub const EMBED_DIGEST: &str = "sha256:85462619ee72";

pub struct Fixture {
    dir: PathBuf,
    pub kernel: Kernel,
    _host: Arc<ikigai_layers::Host>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The grants a person holds to use every view and action.
pub fn person_scopes() -> Vec<String> {
    let mut scopes = Vec::new();
    for layer in ["inquiry-b", "spam-b"] {
        for verb in ["read", "out", "take"] {
            scopes.push(format!("urn:cap:layer:{verb}:{layer}"));
        }
    }
    scopes.push("urn:cap:layer:read:holding-b".into());
    scopes.push("urn:cap:layer:say:examples-b".into());
    scopes.push("urn:cap:layer:out:intents-b".into());
    scopes.push("urn:cap:test:embed".into());
    scopes
}

pub fn person(principal: &str) -> Capability {
    Capability::scoped(person_scopes())
        .with_principal(principal)
        .unwrap()
}

pub fn edge() -> Capability {
    Capability::scoped(
        ["inquiry-b", "spam-b", "holding-b"].map(|l| format!("urn:cap:layer:out:{l}")),
    )
    .with_principal(EDGE)
    .unwrap()
}

/// Reads everything, for the tests' own checks.
pub fn auditor() -> Capability {
    Capability::scoped(
        [
            "inquiry-b",
            "spam-b",
            "holding-b",
            "examples-b",
            "intents-b",
        ]
        .map(|l| format!("urn:cap:layer:read:{l}")),
    )
    .with_principal("urn:collab:participant:auditor")
    .unwrap()
}

fn write(dir: &std::path::Path, path: &str, text: &str) {
    std::fs::write(dir.join(path), text).unwrap();
}

pub fn fixture(name: &str, mount: impl FnOnce(Mount) -> Mount) -> Fixture {
    let dir = std::env::temp_dir().join(format!("ikigai-inquiry-{name}-{}", std::process::id()));
    let (config, state) = (dir.join("config"), dir.join("state"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(config.join("layers")).unwrap();
    std::fs::create_dir_all(config.join("collabs")).unwrap();
    write(&config, "layers/inquiry.context.jsonld", vocab::CONTEXT);
    write(&config, "layers/inquiry.shapes.ttl", vocab::INQUIRY_SHAPES);
    write(&config, "layers/example.shapes.ttl", vocab::EXAMPLE_SHAPES);
    write(&config, "layers/intent.shapes.ttl", vocab::INTENT_SHAPES);
    for layer in ["inquiry-b", "spam-b", "holding-b"] {
        write(
            &config,
            &format!("layers/{layer}.toml"),
            "model = \"tuplespace\"\nshapes = \"inquiry.shapes.ttl\"\n\
             context = \"inquiry.context.jsonld\"\n",
        );
    }
    write(
        &config,
        "layers/intents-b.toml",
        "model = \"tuplespace\"\nshapes = \"intent.shapes.ttl\"\ncontext = \"inquiry.context.jsonld\"\n",
    );
    write(
        &config,
        "layers/examples-b.toml",
        "model = \"channel\"\nshapes = \"example.shapes.ttl\"\ncontext = \"inquiry.context.jsonld\"\n",
    );
    write(
        &config,
        "participants.ttl",
        "<urn:collab:participant:brian> a <http://xmlns.com/foaf/0.1/Person> .\n\
         <urn:collab:participant:chris> a <http://xmlns.com/foaf/0.1/Person> .\n\
         <urn:collab:participant:auditor> a <http://xmlns.com/foaf/0.1/Person> .\n\
         <urn:collab:participant:edge> a <http://www.w3.org/ns/prov#SoftwareAgent> ; \
         <http://www.w3.org/ns/prov#actedOnBehalfOf> <urn:collab:participant:brian> .\n",
    );
    let mut manifest = String::new();
    for (layer, person) in [
        ("inquiry-b", "[\"read\", \"out\", \"take\"]"),
        ("spam-b", "[\"read\", \"out\", \"take\"]"),
        ("holding-b", "[\"read\"]"),
    ] {
        manifest.push_str(&format!(
            "[[mount]]\nlayer = \"{layer}\"\nbrian = {person}\nchris = {person}\n\
             edge = [\"out\"]\nauditor = [\"read\"]\n\n"
        ));
    }
    manifest.push_str(
        "[[mount]]\nlayer = \"examples-b\"\nbrian = [\"say\"]\nchris = [\"say\"]\n\
         auditor = [\"read\"]\n\n[[mount]]\nlayer = \"intents-b\"\nbrian = [\"out\"]\n\
         chris = [\"out\"]\nauditor = [\"read\"]\n",
    );
    write(&config, "collabs/b.toml", &manifest);
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
    // A local embedder stand-in on the `urn:llm:embed` contract: `[len, 1]` for any text.
    let embedder = EndpointSpace::new().bind(
        Exact::new(EMBEDDER),
        FnEndpoint::new("test-embed", |inv| {
            let model = inv.inline_str("model")?;
            let text = inv.inline_str("text")?;
            let body = json!({
                "vector": [text.len() as f32, 1.0],
                "model": format!("{model}:latest"),
                "digest": EMBED_DIGEST,
            });
            Ok(Representation::new(
                ReprType::new("application/json"),
                body.to_string().into_bytes(),
            ))
        }),
    );
    let client = ikigai_inquiry::space(mount(Mount::new(Layers::for_owner("b").unwrap())));
    let spaces: Vec<Arc<dyn Space>> = vec![
        Arc::new(client),
        Arc::new(embedder),
        Arc::new(ikigai_layers::space(&host)),
    ];
    let kernel = Kernel::new(Arc::new(Fallback::new(spaces))).with_clock(Arc::new(SystemClock));
    Fixture {
        dir,
        kernel,
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

/// What the edge would drop: one inquiry, into `layer`.
pub struct Arrival<'a> {
    pub id: &'a str,
    pub site: &'a str,
    pub text: &'a str,
    pub reply_to: Option<&'a str>,
    pub reason: &'a str,
    pub received: &'a str,
}

pub fn arrive(f: &Fixture, layer: &str, a: Arrival<'_>) {
    let mut node = json!({
        "@id": format!("urn:layer:{layer}:inquiry:{}", a.id),
        "@type": format!("{INQ}Inquiry"),
        format!("{INQ}inquiry"): a.id,
        format!("{INQ}site"): a.site,
        format!("{INQ}text"): a.text,
        format!("{INQ}sender"): format!("hash-of-{}", a.id),
        format!("{INQ}reason"): a.reason,
        format!("{INQ}received"): {
            "@value": a.received,
            "@type": "http://www.w3.org/2001/XMLSchema#dateTime"
        },
    });
    if let Some(reply_to) = a.reply_to {
        node[format!("{INQ}replyTo")] = json!(reply_to);
    }
    issue(
        &f.kernel,
        &edge(),
        Verb::Sink,
        &format!("urn:layer:{layer}:tuples"),
        &[("content", &node.to_string())],
    )
    .expect("the shapes admit what the edge drops");
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

/// The entries of the examples channel.
pub fn examples(f: &Fixture) -> Vec<Value> {
    let rep = issue(
        &f.kernel,
        &auditor(),
        Verb::Source,
        "urn:layer:examples-b:entries",
        &[("as", "application/ld+json")],
    )
    .unwrap();
    let doc: Value = serde_json::from_slice(&rep.bytes).unwrap();
    doc["@graph"].as_array().cloned().unwrap_or_default()
}

pub fn act(f: &Fixture, cap: &Capability, action: &str, id: &str) -> Result<String, Error> {
    issue(
        &f.kernel,
        cap,
        Verb::Sink,
        &format!("urn:iki:inquiry:action:{action}"),
        &[("inquiry", id)],
    )
    .map(text)
}
