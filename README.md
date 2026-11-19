# ikigai-inquiry

Inquiries from public contact forms, as [ikigai](https://github.com/ikigai-rs) resources.

A contact form on any of your sites posts to one intake. Instead of mailing you each message,
the module scores it and puts it into a shared space, a layer, that you and the tools you
grant can work through:

- **inquiry**: messages for a person, each marked with the site it came from;
- **spam**: what the filter is confident about, reviewed once a day;
- **holding**: the borderline ones, which a holding stage checks against spam you have
  already confirmed, and only then asks a local model about;
- **examples**: the spam you confirmed, which is what the holding stage compares with.

Every action is a resource too (not spam, confirm as spam, block the sender, respond), and the
views and actions are a client any host can mount. Senders are blocked by a keyed hash of
their address, never the address itself, and nothing tracks an individual.

**Status:** the edge half, the holding stage and the layers client are built. The design is
ledger #3680; nothing is published yet.

## Resource names

Names are singular and type first; each partition has a named key. For
`Layers::for_owner("brian")`, the shared layers are:

| Purpose | Layer name |
|---|---|
| Inquiry | `inquiry-brian` |
| Spam | `inquiry-spam-brian` |
| Holding | `inquiry-hold-brian` |
| Confirmed example | `inquiry-example-brian` |
| Intent | `inquiry-intent-brian` |

The layer host accepts at most 24 lowercase letters, digits or hyphens. Owners of
1–8 characters use `inquiry-`; owners of 9–12 characters use the shorter shared prefix
`inq-` for all five layers so `inq-example-{owner}` fits. Owners start with a letter or
digit. Longer owner identities need explicitly configured layer names.
`Policy::for_owner` uses the same names and validation.

The client's data resources are `urn:iki:inquiry:data:pending:site:{key}` (the site encoded
as lowercase UTF-8 hex) and `urn:iki:inquiry:data:spam:date:{day}:part:{part}` (`site`,
`reason` or `item`). Templates and rendered parts share `urn:iki:inquiry:template:name:{name}`:
`pending-section`, `pending-row`, `spam-body`, `spam-row`, `count-row`,
`pending-section-rendered` and `spam-body-rendered`. The iterator is
`urn:iki:inquiry:function:name:each`; shapes are `urn:iki:inquiry:shape:name:{name}`
(`inquiry`, `spam-example`, `intent`).

Within the layer host's `urn:layer:{layer}:` namespace, inquiry subjects use
`inquiry:id:{id}`, moved copies add `:move:{seq}`, and intents use
`intent:name:{intent}:inquiry:{id}:occurrence:{suffix}`. The edge store uses `block.json`
and `count-date-{date}.json`. This unpublished crate provides no aliases for previous names.

## The edge half

What runs where a form posts. A host opens an `Edge` over its 32-byte sender-key file, a store
and a `Config` over one owner's `Layers`, and mounts `ikigai_inquiry::edge_space`:

| resource | verb | what it does |
|---|---|---|
| `urn:iki:inquiry:route` | Sink | scores an inquiry at `urn:bayes:{model}:score` and drops ONE `inq:Inquiry` tuple into the layer the policy picks (ham → inquiry, unsure → holding, spam → spam) |
| `urn:iki:inquiry:sender-key` | Source | the keyed BLAKE3 key of an `address=` or a `domain=` |
| `urn:iki:inquiry:block:key:{key}` | Exists / Source / Delete | the door's check (`NotFound` once expired), the block's record, lifting it |
| `urn:iki:inquiry:apply` | Sink | the airlock's handler: takes the intents the client wrote and trains, untrains, blocks or unblocks |
| `urn:iki:inquiry:count:date:{date}` | Source | per site: inquiries routed to each layer, blocked drops |
| `urn:iki:inquiry:count:kind:confusion:date:{date}` | Source | score class × a person's label |

Each door declares its own capability (`urn:cap:inquiry:*`); the crate docs list them, the
tuple's properties and what the router does when the classifier cannot answer. The tuple is in
the terms the shipped inquiry shapes admit, so the client reads and moves what the edge wrote.

## The holding stage

`urn:iki:inquiry:stage:name:holding` is a Sink a host runs on a schedule with `min-age=` from
its policy (`holding_space(Policy::for_owner("brian")?)`). Each pass takes every holding
inquiry at least that old, once, by its subject (two workers may run; the second is told who
took it first and moves on), compares it with the confirmed spam examples the same embedding
model made, asks a local model only when none is close, and sends it to spam or inquiry. Any
failure sends it to inquiry marked `unjudged`, never back. A shadow switch records what it
would do and sends everything to inquiry.

The confirmed examples are the `inq:SpamExample` entries of the examples channel, which the
layers client's `confirm-spam` writes. Examples retain the confirmed text, never the sender's
address, name, or sender key; sender blocking belongs to the block set
(`urn:iki:inquiry:block:key:{key}`). An example carries its embedding only when the mount
has an embedder (`Mount::embedder`, the `urn:llm:embed` contract, with the stage's model), and
the stage compares only examples the same model and digest embedded.

## The layers client

A host mounts it with `ikigai_inquiry::space(Mount::new(Layers::for_owner("brian")?))` and
adds only routing and sign-in. Per mount: whose layers (`Layers`), which identity acts
(`Mount::acting_as`), which views show (`Mount::views`), where buttons post
(`Mount::action_base`, default `/inquiry/action/name/`), and an optional local embedder (`Mount::embedder`).

| resource | verb | what it is |
| --- | --- | --- |
| `urn:iki:inquiry:view:name:pending` | Source | the pending inquiries, by site, newest first |
| `urn:iki:inquiry:view:name:spam` | Source | one day's spam (`day=YYYY-MM-DD`, today by default): arrivals by site and by what sent them there, one-line excerpts |
| `urn:iki:inquiry:action:name:remove` | Sink | take from spam; no training, no example |
| `urn:iki:inquiry:action:name:not-spam` | Sink | out to inquiry, take from spam, intent `train-ham` |
| `urn:iki:inquiry:action:name:confirm-spam` | Sink | an example in the examples channel (embedded when the mount has an embedder), take from spam, intent `train-spam` |
| `urn:iki:inquiry:action:name:flag-spam` | Sink | out to spam, take from inquiry, intent `train-spam` |
| `urn:iki:inquiry:action:name:block`, `…:unblock` | Sink | an intent for the edge's block set |
| `urn:iki:inquiry:action:name:respond` | Sink | take from inquiry: handled (the view offers the `mailto:` link) |

The views carry no inline style, so a host can serve them under a strict CSP (`style-src 'self'`). A host styles one class: `.inquiry-message { white-space: pre-line; }` keeps a message's line breaks in its "Full message" disclosure; without it the text still reads, run together.

Every action takes `inquiry=<id>` and runs under the caller's own grant. A move is OUT the new
copy, THEN TAKE the old, so a failure between the two leaves the inquiry in both layers, never
in neither; an action on an inquiry another action took is a Conflict naming who took it, and
writes nothing. Intents go to the owner's intents layer only after the take succeeded.

The layer files a deployment needs are in [`layers/`](layers): one JSON-LD context for every
layer, and closed shapes for the inquiry, spam and holding tuplespaces, the intents tuplespace
and the examples channel. The tests open a real `ikigai-layers` host on exactly these files.

## The vocabulary

[`src/vocabulary.ttl`](src/vocabulary.ttl) (`vocab::VOCABULARY`) is the `inq:` vocabulary, the
document served at <https://ikigai-rs.dev/ns/inquiry>. Published terms come first: the layers'
stamps are `dcterms:created` and `prov:wasAttributedTo`, and each `inq:` term names the
schema:, dcterms:, prov:, oa: or skos: term it is near (a subproperty or subclass only where
that holds, otherwise a SKOS mapping). `tests/vocabulary.rs` checks that it defines exactly the
`inq:` terms the shipped context and shapes use, with their ranges. Its `owl:versionInfo` is
the crate version; it is deployed by hand, and ikigai-devtools' `ns-drift` compares the live
copy with this repository's main, so redeploy after a release that changes it.

## Accessibility

The layers client's two views and its action messages, the holding stage's plain-text answer
and the edge's short plain-text answers are what this crate emits for people to read.
[ACCESSIBILITY.md](ACCESSIBILITY.md) says which rules they follow, how they are tested and
what is not yet known, and how to report a barrier: an issue carrying the `accessibility`
label.

## License

MIT or Apache-2.0, at your option.
