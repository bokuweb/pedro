# Porting chatbook to a native GPUI application

- Kind: implementation plan
- Scope: what pedro has to grow to be [chatbook](https://github.com/skanehira/chatbook)
  without a server, and in which order
- Last updated: 2026-08-22

chatbook is a self-hosted PDF reader: you drag-select a passage of a technical
book, ask about it, and the answer streams back with citations that resolve to
page numbers you can jump to. It runs on Cloudflare — React in the browser,
Hono on Workers, D1 for metadata, R2 for the files, an OpenAI-compatible API
for the answers.

Pedro is the same reader as a native macOS application, with every one of those
four remote pieces replaced by something local:

| chatbook | pedro |
| --- | --- |
| React + pdf.js in a browser | GPUI + pdfium |
| D1 (SQLite over HTTP) | SQLite on disk |
| R2 | files under the application support directory |
| OpenAI-compatible API + `LLM_API_KEY` | an agent CLI already installed and authenticated |
| login, sessions, `AUTH_*` secrets | nothing — the desktop account is the boundary |

The last row is what makes this worth porting rather than deploying: chatbook's
single-user design, its login, and its "bring your own API key" requirement are
all consequences of being a web app. A desktop app owned by one person needs
none of them, and — following [waku](https://github.com/egoist/waku) — it can
borrow the credentials of a coding agent CLI the user has already installed
instead of asking for a key at all.

## The decisions this plan makes

### PDF: pdfium, loaded dynamically

The README left this open between `pdfium-render` and a `wry` webview running
pdf.js. It is settled here as **pdfium-render**, because a `wry` webview always
composites above GPUI content and cannot be clipped: the selection popover, the
highlight overlay and the chat panel are all things that have to sit over the
page, and none of them could. Everything pdf.js gives chatbook for free — page
rasterisation, per-character boxes for selection, the outline — pdfium exposes
too, only as an API rather than as DOM.

pdfium ships as a shared library rather than a crate. `scripts/fetch-pdfium.sh`
downloads a prebuilt one; `pedro-pdf` binds to it at runtime, looking at
`PEDRO_PDFIUM_PATH`, then `vendor/pdfium/lib`, then next to the executable,
then the system library. No build-time linkage, so a machine without the
library still builds and only fails when a document is opened.

### Storage: SQLite plus a content-addressed file store

`~/Library/Application Support/pedro/`:

```
pedro.sqlite3        library, highlights, conversations, reading positions
documents/<sha256>.pdf
```

Keying the file by the SHA-256 of its bytes reproduces chatbook's most useful
property directly: adding the same book twice is the same book, so reading
position and highlights survive re-adding it, and re-adding it under a new
filename only renames it. The schema follows chatbook's D1 schema closely
enough that its migrations remain readable as documentation of ours.

### The model: a local CLI, not an API key

`pedro-agent` already finds `claude` and `codex`. It grows the other half:
running one and reading its answer as a stream of events.

Both CLIs have a non-interactive JSONL mode, and their event shapes were
recorded from the installed versions rather than guessed:

```
claude -p --output-format stream-json --include-partial-messages --verbose
  {"type":"system","subtype":"init","session_id":…}
  {"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":…}}}
  {"type":"assistant","message":{"content":[{"type":"text","text":…}],"usage":{…}}}
  {"type":"result","subtype":"success","is_error":false,"result":…,"usage":{…}}

codex exec --json
  {"type":"thread.started","thread_id":…}
  {"type":"turn.started"}
  {"type":"item.completed","item":{"type":"agent_message","text":…}}
  {"type":"turn.completed","usage":{…}}
  {"type":"turn.failed","error":{"message":…}}
```

Two shapes, one `AgentEvent` stream: `Started`, `Delta`, `Message`, `Finished`,
`Failed`. The parser is deliberately forgiving — an unrecognised line is
skipped, not an error — because these formats move between CLI releases and a
reader must not stop working when one of them adds a field.

Consequences worth stating plainly:

- **The conversation is ours, not the CLI's.** Every request sends the system
  prompt, the stored turns and the new question, and no CLI session is resumed.
  It costs a little more per turn than resuming would, and in exchange the
  history is the rows in our database rather than state inside someone else's
  process, and the same code drives both CLIs.
- **Tools are off.** `claude --tools ""` for a plain answer, `--tools WebSearch`
  when the reader has web search on. This is the port of chatbook's web-search
  toggle: the capability comes from the CLI rather than from a `web_search`
  tool on a Responses API.
- **`is_error` is not `subtype`.** `claude` reports "Not logged in" as
  `{"subtype":"success","is_error":true}`. The runner reads `is_error`.

### Search: the words and what they mean, in the same SQLite file

chatbook has no search. Pedro grows one because the alternative — a question
can only be asked about a passage the reader has already found — makes a
five-hundred-page book worse to ask about than to read.

Both kinds of search live in the file the library is already in, so there is no
second service to run and nothing to keep in step:

- **Words**: FTS5. `unicode61` cannot segment Japanese — it has no spaces to
  segment on, and the whole of 「エラトステネスのふるいで素数を生成する」 is
  one token to it. So text is cut into overlapping character bigrams before it
  is indexed and before it is searched (東京駅 → 東京, 京駅), which is what
  makes a search inside a word find it.
- **Meaning**: `sqlite-vec`, filled by a static embedding model — a table of
  token vectors, mean-pooled and normalised. 134MB, MIT, no transformer, no
  GPU, and a book of 1,800 passages embeds in about forty seconds on the CPU.

Their scores are not comparable — bm25 counts down from zero, cosine counts up
from minus one — so the two rankings are fused by position (reciprocal rank,
k=60) rather than by score.

Two numbers in this are measured rather than picked, and both exist to let a
search answer *nothing*:

- A nearest-neighbour search always has a nearest. Without a floor, a question
  about primes attached the six least-unrelated pages of a book that never
  mentions them. With this model a question and a passage that answers it score
  0.43–0.81, and a question against another subject 0.06 and below
  (`cargo run -p pedro-search --example similarity`); the floor sits at 0.25, in
  the gap.
The result is that a question about primes carries the pages about primes
(`cargo run -p pedro-core --example context -- "…"`).

### Two cuts of the same passages

The index holds each passage cut two ways, because two different questions are
put to it.

**Character pairs** are how a search for a string finds it inside a longer word:
京駅 finds 東京駅 only if both were cut into the same pairs. That is what a
search box is for.

**Content words** — runs of kanji, runs of katakana, latin words — are how a
question finds what it is about. Japanese writes its content in kanji and
katakana and its grammar in hiragana, so the script boundary is a usable word
boundary without a dictionary: 「素数はどうやって生成する?」 cuts to 素数 and
生成, and the grammar is not in the query at all.

The second cut exists because the first cannot be repaired by weighting. Pairs
manufacture tokens that are rare and meaningless at once, and rarity is the only
thing a weighting has to go on:

| token | passages (of 1,836) | idf |
| --- | --- | --- |
| 動く | 0 | 7.52 |
| の本 | 1 | 6.82 |
| で動 | 6 | 5.57 |
| **runtime** | **30** | **4.08** |
| 素数 | 79 | 3.13 |

Asking 「runtime が edge で動くという話はどの本?」, the fragments of the grammar
outrank the two words the question is about, and bm25 is right to think so:
they *are* rarer. Two filters were tried on top of this and both were measured
worse than none — one failed by length, one failed by subject — before the
numbers above showed why neither could work. Cut into content words, the same
question retrieves the right book first, out of two.

What the second cut loses is a content word written in hiragana — ふるい,
できる. The pairs are still there beside it, and a question with no content
words at all falls back to them. A question that *has* them and matches nothing
does not fall back: it has its answer.

### What retrieval still gets wrong

A single kanji is a word only by accident — 決める, 動く, 話す all reduce to
one — so those are dropped from a question that has something longer to go on,
and kept when they are all it has. A question of nothing but verbs is still
poorly served.

Fusing the two rankings by position is what makes a passage both ways of looking
agree on come first. That only works if both lists go in whole: dropping the
overlap from one side to avoid repeating it scores the passage they both ranked
first as though only one had, and a table of contents that one side liked takes
its place.

### Shelves: a question put to several books at once

A question could only be put to a passage the reader had already marked, which
makes a five-hundred-page book worse to ask about than to read — and makes a
question that spans two books impossible to ask at all.

A **shelf** is books gathered so they can be asked together, the way a notebook
gathers sources. Clicking one opens it: the books on it in the middle, the
conversation with it on the right. Nothing is excerpted, because there is no one
book to excerpt; searching every book on the shelf for the question is what
produces the context.

Both kinds of question run the same course — the same turns, the same streaming,
the same citations — and differ only in what context is gathered and where the
reader finds the conversation again. That difference is named once, as
`Conversation` and `Subject`, rather than assumed in a dozen places.

Three decisions worth stating:

- **Shelves are flat.** A question put to a tree would have to say how deep it
  goes, which is a thing to explain and a thing to get wrong, in return for an
  arrangement a library this size does not need.
- **Deleting a shelf keeps its books.** An arrangement of the library is not
  part of it, so `folder_id` is set to null rather than cascading. The
  conversation with the shelf does go, because it was the shelf's.
- **The model is never asked which book it is quoting.** A source is resolved by
  looking its quotation up in each book on the shelf until one holds it. A title
  the model copies slightly wrong would cost the reader the jump; a quotation it
  copies slightly wrong is already handled, by the fragment matching the
  citation lookup has always done.

A conversation belonged to a highlight, so `chat_messages.highlight_id` was
`NOT NULL`. SQLite will not drop that, so the table is rebuilt and its rows
carried across in one transaction. A reader's conversations are the part of that
file that cannot be rebuilt from their PDFs, which is why that migration has a
test of its own and was also run against a copy of a real library.

### Spreads: two pages, the way the book was printed

A printed spread is not "pages 1 and 2". Page 1 is a right-hand page with
nothing facing it, and the pairs run 2–3, 4–5 from there; pairing from the front
instead puts every spread half a book out of step with the paper it is a picture
of. So the cover has a row to itself, held open by a blank of its own width so
the pages below do not slide sideways as the reader scrolls past it.

Which pages face each other is the only thing that knows about spreads. The
scrolling list counts in rows, everything else counts in pages, and one row can
now hold two of them — so the conversion lives in one place with tests, and the
places that scroll (opening a book, turning a page, following a citation) ask it
for a row rather than assuming the page number is one.

The layout is stored per book rather than per reader, because it is a property
of the book: a scanned spread wants it and a slide deck does not.

**A book is not all one shape.** A page turned sideways among upright ones is a
fold-out: it *is* the spread, and pairing it with the page after it gives the
reader half a plan beside an unrelated page of text. So a sideways page takes a
row to itself and the whole width, and so does the page it would have faced —
which keeps every later pair on the same side of the book as before.

**What a page is measured against is the page itself, from the moment the book
opens.** The first version measured the first page and used that until each page
arrived at its true shape, which moved the row while the reader was in it. Every
size is now read in one pass at open: asking the page table costs about seven
milliseconds for a five-hundred-page book, where asking the pages themselves
costs seven milliseconds *each*. The window is divided into columns before
anything is drawn, and a page that does not fill its column is centred in it, so
nothing moves as the pixels arrive.

**Two pages need twice the width, and the window does not grow to meet them.**
A spread drawn at the size a single page had runs off the right edge; shrunk to
fit, it gets smaller the narrower the window is, which is the wrong thing to
take from a reader who is already short of room. So a window with no room for
two pages shows one, and shows two again when there is room — closing a panel or
zooming out is another way to make room. What the reader asked for is what is
remembered; what there is room for is what is drawn.

**What this cost, and what it turned up.** The first version answered "page 1"
for a row past the end of the book. The scrolling list measures itself with
ranges beyond the last row, every frame — so every frame the reader was reported
to be back at the cover, which moved the place, which saved it, which drew
again. A flutter that settled in half a second at startup became a loop that
never settled: 336 redraws in twelve seconds against 72 in half a second before
it. A row past the end holds no page, and saying so is what stopped it.

## Layout

```
crates/
  pedro-agent   discovery (done) + invocation of the agent CLIs
  pedro-pdf     pdfium: pages, rasterisation, text with per-character boxes, outline
  pedro-search  the index: bigram FTS5, vectors, and the fusion of the two
  pedro-core    the domain: store, library, shelves, excerpts, citations, chat
  pedro-app     the GPUI application
```

Only `pedro-app` depends on GPUI. That split is what lets the ported logic —
which is where chatbook's real thinking is — be covered by tests that run
without a window, and it is also what makes the port verifiable on a machine
that cannot yet compile GPUI (see "Verification" below).

## Order of work

Each step leaves the workspace building and tested.

1. ✅ **Workspace** — `pedro-pdf` and `pedro-core`, the pdfium fetch script,
   the application support directory.
2. ✅ **`pedro-pdf`** — open a document, page count and sizes, rasterise a page
   at a scale, extract page text plus per-character rectangles, read the
   outline. Text extraction produces the `\f`-delimited full text chatbook
   stores, so the ported citation lookup works on it unchanged.
3. ✅ **`pedro-core`, storage** — the SQLite schema, adding a document
   (hash, dedup, page count, full text, outline), listing the library, deleting
   a book with its highlights and conversations, reading position.
4. ✅ **`pedro-core`, the ported logic** — `selectExcerpt` (chapter bounds, the
   ±10 page fallback), `buildSystemPrompt`, `buildConversation`/`stripSources`,
   `parseCitations` and `findPageNumber` (whole-quote, page-straddling, and
   fragment matching). A direct port, test for test.
5. ✅ **`pedro-agent`, invocation** — spawn, stream, cancel, plus the two event
   parsers and the tool/web-search options.
6. ✅ **`pedro-core`, chat** — 4 and 5 together: a question about a highlight
   becomes a system prompt, a conversation, a stream of tokens, and finally a
   stored answer with resolved citations. Runnable as
   `cargo run -p pedro-core --example ask`.
7. ✅ **`pedro-search` and retrieval** — the index above, the library-wide
   search box, and the passages a question carries beyond the pages the reader
   marked. Runnable as `cargo run -p pedro-core --example find` and
   `--example context`.
8. ✅ **Shelves** — books gathered onto a shelf, and a question put to the
   shelf as a whole, answered from every book on it with citations that say
   which book as well as which page.
9. ✅ **`pedro-app`** — the screens: library, reader with real pages in a
   continuous scroll, selection and highlights, chat panel with streaming and
   citations, contents, settings, and the keys for turning and zooming. Vim and
   Every part of step 7 is now done.

Steps 1–6 have no GPUI dependency. The workspace is covered by 173 tests, all of
which run without an agent CLI, a network, or a window.

## What building it turned up

Four things were found by running the code rather than by reading about it, and
each shaped the design:

- **pdfium aborts the process when two threads are inside it at once**, with or
  without pdfium-render's `thread_safe` feature. `pedro-pdf` therefore takes a
  process-wide lock for the duration of every call, so callers cannot get this
  wrong; its own test binary runs in parallel as the check that the lock works.
- **Killing an agent CLI does not kill what it started.** The installed
  `claude` is a script around a node process, which kept the pipe open and left
  a cancelled question hanging for as long as the run would have taken. The CLI
  is now started in a process group of its own and the group is signalled, which
  took one cancellation test from 30 seconds to 0.1.
- **A page has two coordinate spaces and pdfium answers in both.** It
  rasterises the crop box and reports characters in media box coordinates. A
  printed book is inset from one to the other, so every mark landed a line above
  its words. Only rendering the page and counting ink inside the box a character
  claimed could settle it; that check is now a test, and
  `cargo run -p pedro-pdf --example boxes` is the tool that found it.
- **Layout state has to be recorded when a frame is painted, not when it is
  laid out.** A scrolling list lays its rows out in its own space and translates
  them on the way to the screen, so bounds taken during layout are in neither
  the space the mouse is reported in nor the space the page is drawn in.

### One connection, one mutex, and the things that must not hold it

The store is a single SQLite connection behind a mutex, which is the right shape
for a reader with one window — and it means anything slow that holds it stops
everything else that needs it.

Two things were slow enough to matter, and both were found the same way: by
timing the log rather than by reading the code.

- **Indexing at startup** held the store for five and a half seconds while it
  cut and embedded eighteen hundred passages. Opening a book needs the same
  store to find the file, so the reader watched a spinner for all of it. The
  cutting and the embedding touch no database; they now happen with nothing
  held, and the store is taken only to read the list and to write each book.
  The book opens in a quarter of a second.
- **Adding a book** did the same indexing inline. It now adds the book and
  indexes afterwards, so a five-hundred-page book is in the library at once and
  searchable a few seconds later.

The rule this leaves is worth stating: *what holds the store must be a query*.
Anything that thinks — embedding, rasterising, waiting on an agent — reads what
it needs, lets go, and comes back to write. The chat already worked this way,
for the same reason and after the same symptom.

## Deliberately not ported

- **Login and sessions.** No `AUTH_*`, no cookies, no revocation story.
- **Multi-device sync.** chatbook's "continue on your phone" comes from the
  server being the source of truth. Pedro's reading position is local.
- **Mobile layout.** The touch selection bar, the chat sheet, the single-column
  breakpoint: all of it exists to serve a phone browser.
- **Token accounting columns.** chatbook records what each answer cost because
  the reader pays per token. A CLI on a subscription does not report a
  comparable number, so the columns are left out rather than filled with zeros.

## Verification

`pedro-agent`, `pedro-pdf` and `pedro-core` build and test with the Command
Line Tools alone. `pedro-app` needs a full Xcode install, because GPUI compiles
Metal shaders with `xcrun metal` during its build (see the README). Steps 1–6
are therefore verified as they land; step 7 waits for Xcode.

### Vim and Emacs keys, and what binding a letter costs

`j` `k` `gg` `G` `/`, and `C-n` `C-p` `C-v` `M-v` `M-<` `M->` `C-s`. Not modes,
and nothing to turn on: a reader who wants neither presses the arrows, which are
still there.

Bound away from the text field **by name**, not by depth. A binding in an
ancestor context fires whenever nothing deeper claims the key, and a text field
claims no plain letters — so `j` bound on the shell alone ate the j out of every
question typed into it. `Pedro && !Input` is what keeps a `j` a `j`, and there
is a test that types "just checking" and reads it back.

### Driving the application itself

gpui opens a headless window in tests, with a real element tree, and delivers
clicks and keystrokes through the same dispatch a reader's go through. So the
shell is tested the way it is used: a real SQLite library in a temporary
directory, real PDFs that pdfium really reads, a stand-in CLI that prints
recorded JSONL, and ⌘⇧S pressed rather than called.

What it cannot do is look. It says which page the reader is on, which pages
share a row, and what a citation resolved to — not whether the result is
legible. Everything in this port that was wrong in a way a test could not see
was found by running the application and reading its log.

Four bugs have turned up in it so far, all of which had been in every build the
reader had used:

- **No key did anything until the reader clicked something.** Keys are
  dispatched along the focus path, and nothing claimed focus when the window
  opened — and the key that focuses the search field is itself a key, so it
  could not be what started them off. The shell now takes focus as the window
  opens.
- **Reopening a book in the same session forgot where the reader was.** The
  place was written to the database and not to the list of books the shell
  holds, and reopening reads that list. It was restored on the next launch,
  which is what made it look like it worked.
- **The reader was on two pages at once, every frame.** The scrolling list asks
  for rows twice a frame and means different things by it: once for the rows it
  is about to draw, and once for a single row at the top, only to find out how
  tall a row is. Taking the reader's place from both wrote two different pages
  to the database every frame and left the page in the status strip flickering.
- **And the first page of the book was drawn and thrown away, forever.** That
  same measuring call asked pdfium for page one, which does not survive being
  filed when the reader is elsewhere — `store` keeps only the pages around them.
  So it was drawn, discarded, and drawn again for as long as the reader sat
  anywhere but the beginning: sixty-four thousand times in the minute it took to
  find. The test hung rather than failed, which is how it was noticed at all.

Nothing in the test suite needs an agent CLI or a network: runs are exercised
against a stand-in that prints recorded JSONL, and the recordings are taken
from the installed `claude` and `codex` rather than written from memory.
