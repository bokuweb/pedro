# pedro

[![CI](https://github.com/bokuweb/pedro/actions/workflows/ci.yml/badge.svg)](https://github.com/bokuweb/pedro/actions/workflows/ci.yml)

A native reader for technical documents that talks to the coding agent CLIs you
already have installed.

Select a passage while reading, ask about it, and the answer streams back with
citations that resolve to page numbers you can jump to. It is a native port of
[chatbook](https://github.com/skanehira/chatbook) — the same reader, with every
remote piece replaced by something local:

| chatbook | pedro |
| --- | --- |
| React + pdf.js in a browser | GPUI + pdfium |
| Cloudflare D1 | SQLite on disk |
| Cloudflare R2 | files under the application support directory |
| OpenAI-compatible API + `LLM_API_KEY` | an agent CLI you have already authenticated |
| login, sessions, `AUTH_*` secrets | nothing — the desktop account is the boundary |

The last row is the point. Following [waku](https://github.com/egoist/waku),
pedro borrows the credentials of a coding agent CLI you already have
(`claude`, `codex`) instead of asking for an API key.

The plan, and the decisions behind it, are in [`docs/PORT_PLAN.md`](docs/PORT_PLAN.md).

## Status

The whole path works: add a book, read it, drag across a passage, ask about it,
and the answer arrives beside the page with sources that turn back to it.

| Area | State |
| --- | --- |
| Agent CLI discovery, and choosing between them | Done |
| Agent invocation: streaming, cancelling, both CLIs' event formats | Done |
| PDF: pages, rasterisation, text with per-character boxes, outline | Done |
| Library: SQLite, content-addressed files, adding and removing books | Done |
| Reader: continuous scroll, zoom, page turning, the place you left off | Done |
| Selecting a passage by dragging, marks that stay on the page | Done |
| Asking about a passage, the answer streaming in beside it | Done |
| Sources that turn to the page they came from | Done |
| Reopening a conversation from the mark that started it | Done |
| Contents, Highlights, Agents and Settings panels | Done |
| Keyboard: arrows to turn pages, ⌘± to zoom, ⌘K to search | Done |
| Vim and Emacs key bindings | Not started |
| Two-page spreads | Not started |
| Reading a PDF out of Google Drive | Done |

## Layout

```
crates/
  pedro-agent   Finding the installed agent CLIs, and running one.
  pedro-pdf     Pages, text with per-character boxes, outlines. pdfium.
  pedro-core    The domain: library, excerpts, prompts, citations, chat.
  pedro-drive   Fetching a PDF out of Google Drive. The only remote piece.
  pedro-app     The GPUI application.
```

Only `pedro-app` depends on GPUI. That is what lets the ported logic — where
chatbook's real thinking is — be covered by tests that run without a window.

## Requirements

- Rust 1.96 or newer
- **pdfium**, which is a shared library rather than a crate:

  ```bash
  ./scripts/fetch-pdfium.sh
  ```

  It lands in `vendor/pdfium/`, which `pedro-pdf` searches. To use a copy
  somewhere else, set `PEDRO_PDFIUM_PATH` to it or to the directory holding it.
  Nothing links against pdfium at build time, so the workspace builds without
  it and only fails when a document is opened.

- **macOS: a full Xcode install**, for `pedro-app` only. GPUI compiles Metal
  shaders at build time with `xcrun metal`, which the Command Line Tools alone
  do not provide. After installing Xcode:

  ```bash
  sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
  ```

  Without it `cargo build -p pedro-app` fails with `unable to find utility
  "metal"`. The other three crates build and test without Xcode.

- At least one agent CLI installed and authenticated (`claude` or `codex`).

- **Optional, for Google Drive**: an OAuth client of your own, in
  `PEDRO_GOOGLE_CLIENT_ID` and `PEDRO_GOOGLE_CLIENT_SECRET`. Everything else
  works without it; this is what lets a book come from Drive rather than from
  the disk. See [`docs/GOOGLE_DRIVE.md`](docs/GOOGLE_DRIVE.md).

## Checks

What CI runs, and what to run before pushing:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features
cargo test --workspace --all-features
```

CI builds with `RUSTFLAGS=-D warnings`, so anything clippy has to say there is
a failure. It runs on macOS, because `pedro-app` compiles Metal shaders and
builds nowhere else; it fetches pdfium first, since the `pedro-pdf` tests open
real documents. The embedding model is not fetched — nothing in the tests needs
it.

## Using it

```bash
cargo run -p pedro-app
```

Add a PDF with the plus in the sidebar header, or paste a Google Drive link
into the field the button beside it opens. Open it, drag across a passage,
type a question, and press the arrow. The answer streams into the panel beside
the page; the sources under it turn the book to where they came from. The
passage stays marked, and pressing a mark reopens what was asked about it.

| | |
| --- | --- |
| Turn pages | `←` `→` |
| Zoom | `⌘-` `⌘=` `⌘0` |
| Search the sidebar | `⌘K` |
| Choose which CLI answers | Agents, in the sidebar under More |
| Stop an answer | Stop, under the answer being written |
| Remove a book or a mark | Remove on its row, which asks twice |

Ask a question about a real PDF without opening a window — the whole reading
pipeline except the screens:

```bash
cargo run -p pedro-core --example ask -- book.pdf 12 "この節の要点は?"
```

It adds the book to your library, marks the top of page 12, sends the chapter
around it to whichever CLI it finds, and prints the answer with its sources.

Check CLI discovery on its own:

```bash
cargo run -p pedro-agent --example detect
```

## Where things are kept

```
~/Library/Application Support/pedro/
  pedro.sqlite3          library, highlights, conversations, reading positions
  documents/<sha256>.pdf
```

Books are keyed by the SHA-256 of their bytes, so adding the same book twice is
the same book: its highlights and your place survive re-adding it, and re-adding
it under a new filename only renames it.

## How CLI discovery works

A GUI application launched from Finder or Dock inherits `launchd`'s
environment, not the `PATH` the user's shell builds. Agent CLIs are usually
installed somewhere only the shell knows about, so `pedro-agent`:

1. reads the inherited `PATH`,
2. asks the login shell for its `PATH` (`$SHELL -lic`, with the value wrapped in
   markers so rc-file output can be filtered out),
3. adds well-known bin directories that are often missing from both,
4. probes each candidate with `--version`.

Every subprocess is bounded by a timeout, so a hung CLI cannot stall startup.
Discovery runs off the UI thread and the result is shown in the top bar.

## How a question is answered

1. The highlight names a page; the page names a chapter, taken from the book's
   own outline. That chapter is the context — not the whole book, which would
   be slow and often impossible, and not the highlighted sentence, which
   answers nothing. A book with no outline sends ten pages either side.
2. The system prompt carries the chapter and the highlighted passage, and asks
   for a `## Sources` section.
3. The whole conversation is sent every time. No CLI session is resumed: the
   history is rows in pedro's database rather than state in someone else's
   process, and the same code drives both CLIs.
4. Tools are off — `claude --tools ""`, or `--tools WebSearch` when web search
   is on, which is how chatbook's web-search toggle is ported. A coding agent
   left with its tools will go reading your filesystem to answer a question
   about a book.
5. Each source's quoted passage is looked up in the book's text to find its
   page. A passage the model reworded is searched again in fragments; one that
   is nowhere in the book says so, which is the only hint that it was not
   quoted verbatim.

## Testing

```bash
cargo test --workspace --exclude pedro-app
```

`pedro-pdf`'s tests need pdfium (`./scripts/fetch-pdfium.sh`) and fail with
instructions rather than skipping when it is missing. Nothing needs an agent
CLI: runs are tested against a stand-in that prints recorded JSONL, so a
credential-less machine still covers streaming, refusals and cancellation.
