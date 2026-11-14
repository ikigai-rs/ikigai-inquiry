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

**Status:** scaffold. The design is ledger #3680; the module is not built or published yet.

## Accessibility

This crate emits nothing people read yet. [ACCESSIBILITY.md](ACCESSIBILITY.md) says so, grows with the code, and says how to report a barrier: an issue carrying the `accessibility` label.

## License

MIT or Apache-2.0, at your option.
