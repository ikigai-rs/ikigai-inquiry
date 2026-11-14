# Accessibility

What `ikigai-inquiry` emits that people read, and how to report a barrier. It is not a
conformance claim. Adapted from the
[ACCESSIBILITY.md template](https://github.com/mgifford/ACCESSIBILITY.md) (MIT).

| Field | Value |
|---|---|
| Target | WCAG 2.2 Level AA, for any page that presents this repository's output: the layers client's two views and its action messages |
| Status | Evaluation in progress. No conformance claim. |
| Owner | the maintainers of this repository |
| Report a barrier | [a GitHub issue](https://github.com/ikigai-rs/ikigai-inquiry/issues/new?labels=accessibility) with the `accessibility` label; anything security-sensitive through [private vulnerability reporting](https://github.com/ikigai-rs/ikigai-inquiry/security/advisories/new) |
| Last reviewed | 2026-11-14 |

## What it emits that people read

The layers client (`src/view.rs`, `src/action.rs`). The edge and the holding stage are not
built yet and emit nothing.

| Output | Where | What a person reads |
|---|---|---|
| The pending inquiries | `urn:iki:inquiry:view:pending`, a whole HTML page | a count, then a section per site, each inquiry newest first: an excerpt as its heading, when it arrived, the reply address, the full message behind a disclosure, a reply link, and buttons |
| The day's spam | `urn:iki:inquiry:view:spam`, a whole HTML page | a count, two tables (arrivals by site, by what sent them to spam), then each arrival: an excerpt as its heading, site, reason, time, and buttons |
| An action's answer | `urn:iki:inquiry:action:{action}`, one line of `text/plain` | what was done, for example "Moved inquiry a1 from spam to your inquiries." A refusal is the kernel's typed error. |

### The rules the views follow

- **Meaning in words.** Every button and link says what it does in words ("Mark handled",
  "Move to spam", "Block sender", "Not spam", "Confirm as spam", "Remove", "Write a reply").
  Nothing is carried by color, icon or position alone; the pages ship no styles and no script.
- **Structure to navigate by.** `lang="en"`, a `<title>`, one `<main>` and one `<h1>`; an
  `<h2>` per site (or per summary table) and an `<h3>` per inquiry, never skipping a level;
  each inquiry an `<article>` labelled by its heading, in an ordered list; the actions of
  one inquiry a labelled group.
- **Repeated controls tell themselves apart.** Every inquiry has the same buttons, so each
  button and the reply link is described by its inquiry's heading (`aria-describedby`):
  "Mark handled" is announced with the excerpt it acts on.
- **Tables.** Column and row headers are `<th>` with `scope`; each table is labelled by
  its section heading.
- **Times.** Shown as `2026-11-14 09:30 UTC` and given machine-readable in `<time datetime>`.
- **A stranger's text stays text.** It reaches a page only through `$h{…}` (HTML-escaped).
  An excerpt is one line the module writes itself, at most 100 characters, with line breaks,
  controls and invisible direction marks removed. The full message keeps its line breaks
  and is a direction isolate (`dir="auto"`), so a right-to-left override in it cannot
  reorder the page around it.
- **A reply link only to a plain address.** An address that is not one plain
  `local@domain` is shown as text, and the link says in words that it cannot be used.
- **Empty states in words**: "No inquiries are waiting." and "No spam arrived on
  2026-11-14 that is still waiting."
- Every button is a real `<button>` in a `<form method="post">`, so it works from the
  keyboard and without script.

### How it is tested

`tests/views.rs` renders both views against a real layer host and, on every page, parses it
as XML (well-formedness) and checks: `lang`, a non-empty `<title>`, one `<main>`, one `<h1>`,
no skipped heading level, unique ids, every `aria-labelledby` and `aria-describedby` naming
an element that exists, every button and link having words, every form posting, every
`<th>` having a `scope`. It also checks that hostile text (markup, quotes, a template
marker, a right-to-left override, an address built to break out of an attribute) reaches
the page only escaped. `tests/actions.rs` pins each action's message.

### Not yet known

No screen reader, magnifier or voice-control user has used these pages, and no automated
WCAG engine (axe, for example) has run on them: the checks above are structural. Contrast
and focus visibility belong to the host's styles, since these pages ship none. Times are in
UTC, not the reader's zone.

## Contributing

Read this file before adding or changing any output people read: messages, pages, terminal
text. Add that output to this file in the same pull request, keep meaning in words, and say in
the pull request what was tested, with what, and what was not.

## Reporting a barrier

[Open an issue](https://github.com/ikigai-rs/ikigai-inquiry/issues/new?labels=accessibility)
with the `accessibility` label (the link sets it when GitHub lets you add labels; otherwise a
maintainer adds it): what you were doing, what happened, and your tools and any assistive
technology, if you are comfortable sharing them. Anything security-sensitive goes through
[private vulnerability reporting](https://github.com/ikigai-rs/ikigai-inquiry/security/advisories/new).
**We acknowledge a report within 7 days**, and promise nothing further.

## Change history

| Date | Change |
|---|---|
| 2026-11-14 | First version. |
| 2026-11-14 | The layers client: the pending and spam views, and the action messages. |
