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

**Status:** the layers client is built; the edge and the holding stage are not. The design is
ledger #3680; nothing is published yet.

## The layers client

A host mounts it with `ikigai_inquiry::space(Mount::new(Layers::for_owner("brian")?))` and
adds only routing and sign-in. Per mount: whose layers (`Layers`), which identity acts
(`Mount::acting_as`), which views show (`Mount::views`), where buttons post
(`Mount::action_base`), and an optional local embedder (`Mount::embedder`).

| resource | verb | what it is |
| --- | --- | --- |
| `urn:iki:inquiry:view:pending` | Source | the pending inquiries, by site, newest first |
| `urn:iki:inquiry:view:spam` | Source | one day's spam (`day=YYYY-MM-DD`, today by default): arrivals by site and by what sent them there, one-line excerpts |
| `urn:iki:inquiry:action:remove` | Sink | take from spam; no training, no example |
| `urn:iki:inquiry:action:not-spam` | Sink | out to inquiry, take from spam, intent `train-ham` |
| `urn:iki:inquiry:action:confirm-spam` | Sink | an example in the examples channel (embedded when the mount has an embedder), take from spam, intent `train-spam` |
| `urn:iki:inquiry:action:flag-spam` | Sink | out to spam, take from inquiry, intent `train-spam` |
| `urn:iki:inquiry:action:block`, `…:unblock` | Sink | an intent for the edge's block set |
| `urn:iki:inquiry:action:respond` | Sink | take from inquiry: handled (the view offers the `mailto:` link) |

Every action takes `inquiry=<id>` and runs under the caller's own grant. A move is OUT the new
copy, THEN TAKE the old, so a failure between the two leaves the inquiry in both layers, never
in neither; an action on an inquiry another action took is a Conflict naming who took it, and
writes nothing. Intents go to the owner's intents layer only after the take succeeded.

The layer files a deployment needs are in [`layers/`](layers): one JSON-LD context for every
layer, and closed shapes for the inquiry, spam and holding tuplespaces, the intents tuplespace
and the examples channel. The tests open a real `ikigai-layers` host on exactly these files.

## Accessibility

The layers client's two views and its action messages are what this crate emits for people to
read. [ACCESSIBILITY.md](ACCESSIBILITY.md) says which rules they follow, how they are tested and
what is not yet known, and how to report a barrier: an issue carrying the `accessibility` label.

## License

MIT or Apache-2.0, at your option.
