# Vector Recall over Archive (spec v0.1)

Status: research draft, accepted as a working direction; not implementation-ready
until the alignment notes below are resolved.
Audience: implementation agent (Codex) and maintainer.
Depends on: `docs/architecture.md` v0.1, `docs/contracts.md` v0.1, `docs/strategy.md`.

> **Alignment note.** This spec was written against `README.md` only. Wherever it names
> identifiers of existing contracts (scope id, record id field, task envelope shape,
> journal entry shape, the canonical text field of an archive record), treat the names
> as placeholders and align them with `docs/contracts.md` and `docs/architecture.md`
> before writing code. Such places are marked `TODO(align)`.
>
> **2026-06-10 maintainer note.** The project now has validated `MemoryUnit`s,
> fidelity review, forgetting, and buffered recall stats. Before implementation,
> align this draft to the current architecture:
>
> - index validated active `MemoryUnit`s, not whole ArchiveEntry JSON blobs;
> - exclude `Forgotten` units from vector recall, and make `remember_back` either
>   re-enable or re-embed the unit;
> - keep the normative core API vector-based (`query_vector` in), with any
>   text-to-vector convenience living in adapters/hosts;
> - implement embedding work through the existing PendingTask / pull-driver
>   pattern, not a parallel task system;
> - integrate vector recall reinforcement with the existing buffered recall stats
>   path instead of writing recall events on every query.
>
> **2026-07-03 maintainer note.** The implementation TZ
> `docs/research/vector-storage-tz-2026-07-03.md` supersedes this draft. In
> particular, the draft's `intfloat/multilingual-e5-small` default was retracted
> after a live fastembed check: `fastembed==0.8.0` does not expose that model via
> `TextEmbedding`. The v1 implementation default is
> `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` (384 dim).

---

## 0. Decision summary

1. The vector store is **embedded in the Rust core** as plain files per scope. No
   external vector database, no managed service, no extra daemon, no Docker container.
2. Vectors are **derived data over the Archive layer**. Archive JSONL remains the only
   source of truth. The vector index can be deleted and rebuilt at any time with zero
   information loss.
3. Embeddings are produced by the **host**, never by the core (provider independence is
   preserved). Indexing goes through a new `PendingTask` kind `embed_batch`, executed
   during sleep. Query-time embedding goes through a host-registered embedder called
   synchronously at recall time.
4. Default embedding model: `intfloat/multilingual-e5-small` (384 dims, ~512-token
   input window) run **locally** via `fastembed` (ONNX, CPU). Good
   Ukrainian/Russian/English quality at small size, cross-lingual (a Ukrainian query
   finds an English memory). The model id is recorded in the index manifest; switching
   models later is a single `rebuild` command.
5. Vector recall is **opt-in per scope**. Default state is `disabled`. Disabling can
   either keep or purge vectors; purge deletes the whole derived directory for that
   scope.
6. Recall is **on-demand**: the library exposes it as one explicit function call
   (hosts typically surface it to their LLM as a tool). It is NOT
   retrieval-augmented generation on every turn. Below a similarity threshold the
   core returns nothing instead of weak matches.
7. MVP index is a **flat (brute-force) cosine index** behind a `VectorIndex` trait.
   HNSW (e.g. `usearch`) is a later drop-in replacement if a scope ever exceeds
   roughly 500k records. Do not implement HNSW now.
8. **Library boundary.** Memory Engine is an embeddable library: structured data and
   text in, structured data and text out. It never renders UI, never talks to chat
   platforms, never emits user-facing states, and never decides assistant wording.
   Sections 3-7 are normative for the library; section 8 is a non-normative example
   of one host and MUST NOT leak into core or adapter code.

## 1. Why these choices

**Scale.** One 384-dim f32 vector is 1,536 bytes. 100,000 distilled archive records
per scope is ~150 MB of vectors and a full brute-force cosine scan in well under
50 ms on one CPU core. Personal-assistant scopes accumulate distilled records in the
low tens of thousands over years. A dedicated vector database solves problems this
project does not have.

**Philosophy.** The project's principles are "memory lives in plain, human-readable
files" and "private by choice". An embedded index in the scope's own directory means:
purge = delete one directory; audit = read `rows.jsonl`; backup = copy files. A
separate server process breaks all three.

**Privacy.** Local embedding model + per-scope opt-in means text from a disabled chat
never gets embedded, never leaves the text files, and never reaches any third party.

## 2. Design goal: recall must feel human, not like database search

Six mechanisms. Each is tagged: `[core]` = implemented and enforced inside the
library; `[host]` = recommended integration pattern, never implemented in the
library.

- **M1. On-demand recall, never every-turn RAG.** `[core + host]` The library only
  exposes recall as an explicit function call and never auto-injects memories into
  every turn (core guarantee). The recommended host pattern is to surface that
  function to the LLM as a tool, so the model itself decides "I should think back"
  mid-turn. Constant every-turn retrieval is the anti-pattern: it makes an assistant
  feel like a machine that instantly knows everything.
- **M2. Scarcity.** `[core]` Recall returns at most `top_k` (default 5) memories
  under a token budget (default ~1,500 tokens), enforced by the core. Humans recall
  a few things, not fifty chunks.
- **M3. Honesty.** `[core]` If the best hit is below `min_sim`, the core returns an
  empty result with a reason instead of weak matches. It never fabricates or pads.
  How the assistant phrases "I don't remember the details" is host prompt design,
  outside the library.
- **M4. Reinforcement.** `[core]` Every memory actually returned gets
  `recall_count += 1` and a fresh `last_recalled_at`. Frequently recalled memories
  rank higher next time ("vivid" memories), like in humans.
- **M5. Consolidation during sleep.** `[core]` New archive records are embedded
  during the existing sleep pipeline, mirroring human memory consolidation. No
  embedding happens in the hot message path.
- **M6. Gentle, reversible fading.** `[core]` Ranking includes recency decay, so
  ancient never-recalled memories sink, but nothing is ever auto-deleted. Forgetting
  stays human-controlled, per `docs/strategy.md`.

## 3. Storage layout (per scope)

```
memory/<scope>/archive/vectors/
  manifest.json      # index metadata, written last on every mutation
  vectors.f32        # raw vector matrix, append-only between compactions
  rows.jsonl         # one line per vector row, same order as vectors.f32
  events.jsonl       # append-only recall/tombstone events, merged at compaction
```

The directory exists only when the scope's vector state is not `disabled`.

### 3.1 `manifest.json`

```json
{
  "version": 1,
  "model_id": "intfloat/multilingual-e5-small",
  "dim": 384,
  "metric": "cosine",
  "normalized": true,
  "rows": 1234,
  "state": "ready",
  "built_at": "2026-06-10T12:00:00Z",
  "updated_at": "2026-06-10T12:00:00Z",
  "backfill_cursor": null
}
```

- `state`: `building | ready | corrupt`.
- `backfill_cursor`: last archive `record_id` already dispatched for embedding during
  backfill; `null` when backfill is complete. `TODO(align)` with how task progress is
  tracked elsewhere (journal vs manifest).

### 3.2 `vectors.f32`

Raw binary, no header. Little-endian IEEE-754 f32, row-major: row `i` occupies bytes
`[i*dim*4, (i+1)*dim*4)`. All stored vectors are L2-normalized (the core normalizes
defensively on write even if the host already did). File size MUST equal
`valid_rows_in_rows.jsonl * dim * 4` (see recovery rule R1).

### 3.3 `rows.jsonl`

One JSON line per row, line number == row index in `vectors.f32`:

```json
{"row": 0, "record_id": "arc_0001", "created_at": "2025-03-12T10:00:00Z", "text_hash": "sha256:...", "recall_count": 0, "last_recalled_at": null}
```

- `record_id` is the archive record id. `TODO(align)` exact field name with
  `contracts.md`.
- `created_at` is copied from the archive record (used for recency decay).
- `text_hash` is sha256 of the full canonical text that was embedded (before
  truncation), so edits to a record can be detected and re-embedded.
- `recall_count` / `last_recalled_at` here reflect the state **as of last
  compaction**; the live values are these plus the overlay from `events.jsonl`.
- `rows.jsonl` and `vectors.f32` are append-only between compactions and are only
  rewritten together during compaction.

### 3.4 `events.jsonl`

Append-only overlay, merged into `rows.jsonl` at compaction:

```json
{"ts": "2026-06-10T12:00:00Z", "type": "recall", "record_id": "arc_0001"}
{"ts": "2026-06-10T12:00:00Z", "type": "tombstone", "record_id": "arc_0002"}
```

- `recall`: increments live `recall_count`, updates live `last_recalled_at`.
- `tombstone`: written when the underlying archive record is deleted or the user
  deletes a memory. Tombstoned rows are skipped at search time and physically removed
  at compaction.

### 3.5 Write protocol, recovery, compaction

- **Append (during sleep):** append vector bytes to `vectors.f32`, then append the
  matching line to `rows.jsonl`, then rewrite `manifest.json` via tmp + atomic rename.
  Journal an entry before and after (`vectors.append` with row range). `TODO(align)`
  journal entry shape.
- **Recovery rule R1 (on open):** let `L` = number of valid lines in `rows.jsonl`.
  If `len(vectors.f32) > L*dim*4`, truncate `vectors.f32` to that size (crash between
  the two appends). If `len(vectors.f32) < L*dim*4`, set `state = corrupt`; the only
  exit from `corrupt` is `rebuild`. Rebuild is cheap because everything is derived.
- **Compaction:** triggered at sleep when (`tombstone events > 0`) or
  (`events.jsonl lines > 1000`). Rewrites `vectors.f32` + `rows.jsonl` without
  tombstoned rows and with events merged, via tmp files + atomic rename, manifest
  last, journal entries around it. `events.jsonl` is truncated after a successful
  compaction.
- **Assumption A1:** exactly one engine process owns a memory root at a time (current
  architecture). No cross-process file locking in MVP; note it in
  `docs/architecture.md` limitations.

## 4. Scope state machine and configuration

Per-scope state `vector_recall`: `disabled` (default) | `enabled`. Stored where other
per-scope settings live. `TODO(align)` exact location (scope manifest vs config).

Transitions:

- `disabled -> enabled`: create `vectors/` dir, manifest `state = building`, start
  backfill (emit `embed_batch` tasks for all live archive records, in chunks, advancing
  `backfill_cursor`). When the cursor reaches the end, `state = ready`. New records
  promoted during `building` are also embedded.
- `enabled -> disabled (keep)`: stop emitting embed tasks; `recall_deep` returns
  `found: false, reason: "disabled"`. Files stay on disk.
- `enabled -> disabled (purge)`: same, plus recursive delete of
  `memory/<scope>/archive/vectors/`. Journal records the purge. Archive text is
  untouched.
- `rebuild`: purge + enable. Also the required path after a `model_id` change or a
  `corrupt` state.

Global defaults in config (example, `TODO(align)` with the config layout in
`config/`):

```toml
[vector_recall]
default_state    = "disabled"
model_id         = "intfloat/multilingual-e5-small"
dim              = 384
top_k            = 5
min_sim          = 0.75   # MUST be calibrated, see section 11
vivid_sim        = 0.85   # label threshold "vivid" vs "faint"
recall_token_budget = 1500
half_life_days   = 180
w_recency        = 0.10
w_strength       = 0.05
embed_batch_size = 64
embed_input_token_cap = 480
cache_max_scopes = 64    # in-RAM vector caches kept at once (LRU), for many-scope hosts
```

## 5. Task contract: `embed_batch`

New `PendingTask` kind emitted by the core during sleep for enabled scopes.
`TODO(align)` the task envelope (id field names, status flow) with the existing
PendingTask contract; only the payload below is new.

Request payload:

```json
{
  "kind": "embed_batch",
  "scope": "<scope_id>",
  "model_id": "intfloat/multilingual-e5-small",
  "dim": 384,
  "items": [
    {"record_id": "arc_0001", "text": "<canonical archive text, truncated to embed_input_token_cap>"}
  ]
}
```

Result payload submitted by the host:

```json
{
  "model_id": "intfloat/multilingual-e5-small",
  "dim": 384,
  "results": [
    {"record_id": "arc_0001", "vector": [0.0123, -0.0456, "..."]}
  ]
}
```

Rules:

- Batch size <= `embed_batch_size`.
- Text sent for embedding is the canonical text field of the archive record
  (`TODO(align)` field name), truncated to ~`embed_input_token_cap` tokens using the
  engine's token estimator (`TODO(align)`; fallback heuristic `tokens ~= ceil(chars/3)`
  for mixed Ukrainian/English). `text_hash` is computed over the full text.
- **Model family prefixes:** e5-family models require `"passage: "` prefix for
  indexed texts and `"query: "` for queries. This is the host's responsibility; with
  fastembed use `passage_embed()` / `query_embed()`, which apply prefixes for e5
  models. Verify against the installed fastembed version; if absent, prepend
  prefixes manually. Non-e5 models (e.g. paraphrase-multilingual-MiniLM) use no
  prefixes.
- Host SHOULD return L2-normalized vectors; the core MUST normalize defensively
  anyway.
- On submit, the core validates `model_id` and `dim` against the manifest. Mismatch
  -> reject the result, log, keep state (invariant I7).
- Failed/expired tasks are simply re-emitted at the next sleep (idempotent: a record
  already present in `rows.jsonl` with matching `text_hash` is skipped at emission
  time).

## 6. Recall API and scoring

### 6.1 Rust core

```rust
pub enum VectorState { Disabled, Building, Ready, Corrupt }

pub struct RecallOpts {
    pub top_k: usize,          // default from config
    pub min_sim: f32,
    pub token_budget: usize,
    pub now: Timestamp,        // injectable for tests
}

pub struct RecallHit {
    pub record_id: RecordId,
    pub text: String,          // canonical archive text
    pub created_at: Timestamp,
    pub sim: f32,              // raw cosine
    pub score: f32,            // ranked score, see 6.3
    pub recall_count: u32,     // live value (rows + events overlay)
}

impl MemoryEngine {
    /// Core never embeds. Caller provides the query vector and the model_id it
    /// came from; mismatch with manifest.model_id is an error.
    pub fn recall_deep(&self, scope: &ScopeId, query_vec: &[f32], model_id: &str,
                       opts: &RecallOpts) -> Result<Vec<RecallHit>>;

    pub fn vector_state(&self, scope: &ScopeId) -> VectorState;
    pub fn set_vector_state(&mut self, scope: &ScopeId, enabled: bool, purge: bool) -> Result<()>;
    pub fn rebuild_vectors(&mut self, scope: &ScopeId) -> Result<()>;
}

/// MVP: FlatIndex. Search loads vectors.f32 into memory on first use per scope,
/// caches, invalidates on append/compaction.
trait VectorIndex {
    fn append(&mut self, rows: &[(RecordId, Vec<f32>)]) -> Result<()>;
    fn search(&self, q: &[f32], k: usize, skip: &TombstoneSet) -> Vec<(RowMeta, f32)>;
}
```

### 6.2 Search algorithm (FlatIndex)

1. Normalize `query_vec`.
2. Dot product against every non-tombstoned row (normalized vectors, so dot ==
   cosine). Keep a `BinaryHeap` of the best `4 * top_k` candidates by raw `sim`.
3. Gate: drop candidates with `sim < min_sim`.
4. Rank survivors by `score` (6.3), take `top_k`.
5. Cut to `token_budget` using the token estimator, dropping lowest-score hits first.
6. Side effect: for every hit actually returned, append a `recall` event to
   `events.jsonl` (M4 reinforcement). This is the only write on the read path; it is
   append-only and crash-tolerant (a lost event only loses one reinforcement tick).

### 6.3 Scoring formula

```
recency  = exp(-ln(2) * age_days / half_life_days)      // 1.0 today, 0.5 at half-life
strength = 1 - 1 / (1 + recall_count)                   // 0 never recalled, -> 1
score    = sim + w_recency * recency + w_strength * strength
```

`min_sim` gates on raw `sim`, not on `score`, so recency can never push an irrelevant
memory over the line. If the archive record schema has an importance field
(`TODO(align)`), add `+ w_importance * importance` with default `w_importance = 0.05`.

### 6.4 Disabled / empty semantics

- Scope `disabled` or `building` with zero rows: return empty with reason
  (`disabled` / `building`); adapter maps it to `{"found": false, "reason": ...}`.
- No survivors above `min_sim`: `{"found": false, "reason": "below_threshold"}`.

## 7. Python adapter (PyO3) surface

```python
# Host registers its embedder once at startup. The core stays model-free;
# the adapter holds the callables and uses them for queries and (optionally)
# for executing embed_batch tasks in-process.
engine.register_embedder(
    model_id="intfloat/multilingual-e5-small",
    dim=384,
    embed_passages=fn_texts_to_vecs,   # list[str] -> list[list[float]]
    embed_query=fn_text_to_vec,        # str -> list[float]
)

engine.vector_state(scope) -> dict
# {"state": "ready", "rows": 1234, "model_id": "...", "dim": 384, "built_at": "..."}

engine.set_vector_state(scope, enabled: bool, purge: bool = False) -> None
engine.rebuild_vectors(scope) -> None

engine.recall_deep(scope, query: str, top_k: int | None = None,
                   min_sim: float | None = None) -> dict
# Convenience: embeds query via the registered embed_query, then calls core
# recall_deep. Normative return shape:
# {"found": bool, "reason": str | None,
#  "memories": [{"record_id", "when", "age_days", "strength", "sim", "text"}]}
# Section 8.3 merely shows this same dict passed through to an LLM tool result.
```

The existing task loop (`pending_tasks` / submit result, `TODO(align)` names) gains
the `embed_batch` kind; the host either handles it with the registered
`embed_passages` automatically (adapter helper) or manually.

## 8. Example host integration (non-normative): `hosts/telegram_gemini_bot`

> Nothing in this section is part of the library. It shows one way a host can wire
> the three library primitives (recall function, per-scope toggle, `embed_batch`
> task) into a Telegram bot. A Godot game or any other host wires them differently;
> the library neither knows nor cares. Codex MUST NOT reference anything from this
> section inside `crates/`.

### 8.1 Local embedder (fastembed)

```python
from fastembed import TextEmbedding

MODEL_ID = "intfloat/multilingual-e5-small"
model = TextEmbedding(MODEL_ID)   # downloads ONNX once; cache dir configurable

def embed_passages(texts):  # for embed_batch tasks
    return [v.tolist() for v in model.passage_embed(texts)]

def embed_query(text):      # for recall
    return next(iter(model.query_embed([text]))).tolist()
```

If the installed fastembed registry lacks `multilingual-e5-small`
(check `TextEmbedding.list_supported_models()`), fall back in this order:
`sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` (384 dim, multilingual,
no prefixes) or `BAAI/bge-m3` (1024 dim, heavier, better quality, no prefixes).
Whatever is chosen goes into config `model_id` + `dim` once; everything downstream is
derived.

**Privacy policy (host-level): the embedder for this bot is local-only. Never wire
chat text into a remote embedding API.** (Invariant I5.)

### 8.2 Gemini tool declaration

```json
{
  "name": "recall_distant_memory",
  "description": "Search your own long-term memory for things from much earlier conversations that are not visible in the current context: past events, people, decisions, facts. Do not use it for things already visible in context.",
  "parameters": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "What you are trying to remember, as a short natural-language phrase in the language of the conversation."
      }
    },
    "required": ["query"]
  }
}
```

System prompt addition (translate into the bot's prompt language):

> You have layered memory. The visible context covers the recent conversation and
> core facts about the user. For older things, call `recall_distant_memory`. Whatever
> it returns are your own genuine recollections: weave them in naturally ("right, we
> talked about this back in March..."), mention rough dates, allow for imperfection.
> If it returns nothing, say honestly that you don't remember the details. Never
> invent memories.

### 8.3 Tool execution flow

1. Model emits the function call.
2. Host sends Telegram chat action `typing` (this is the "бот задумався" moment;
   optionally a short flavor message like "хм, зараз пригадаю..." for long recalls).
3. Host: `result = engine.recall_deep(scope, query)`.
4. Tool result returned to the model:

```json
{
  "found": true,
  "memories": [
    {"when": "2025-03-12", "age_days": 455, "strength": "vivid", "sim": 0.87,
     "text": "..."},
    {"when": "2024-11-02", "age_days": 585, "strength": "faint", "sim": 0.78,
     "text": "..."}
  ]
}
```

`strength = "vivid"` if `sim >= vivid_sim`, else `"faint"`. The prompt may tell the
model to hedge faint memories ("if I remember correctly...").

5. If the scope is disabled: `{"found": false, "reason": "disabled"}`; the model says
   it cannot recall that far back in this chat (which is literally true).

### 8.4 User-facing chat commands (digital hygiene, self-service)

Per current chat's scope, executable by chat members themselves:

- `/memory_vectors status` -> state, row count, model.
- `/memory_vectors on` -> enable + backfill (reply with progress note).
- `/memory_vectors off` -> disable, keep files.
- `/memory_vectors purge` -> disable + delete vectors; require an explicit
  confirmation step (second command or inline button) before deleting.

This makes the per-chat opt-in real: in one chat the bot has deep recall, in another
the same bot physically never embeds anything.

## 9. `memory_terminal` commands

- `vectors status <scope>`
- `vectors enable <scope>` / `vectors disable <scope> [--purge]`
- `vectors rebuild <scope>`
- `recall <scope> "query" [--debug]` where `--debug` prints the top 20 raw `sim`
  values and texts regardless of threshold. This is the calibration tool.

## 10. Invariants (each one gets a test)

- **I1.** A `disabled` scope never appears in any `embed_batch` task, and
  `recall_deep` on it returns `found: false`. Its text never leaves the JSONL files.
- **I2.** One index per scope, files only under that scope's directory; search never
  crosses scopes.
- **I3.** Purge recursively deletes `memory/<scope>/archive/vectors/`, journals the
  fact, and leaves archive text untouched.
- **I4.** The index is always rebuildable from archive JSONL alone, deterministically
  for a fixed `model_id`. Losing the index loses nothing but compute.
- **I5.** Host policy for private scopes: local embedder only. The core records
  `model_id` informationally; enforcement is host-side and documented in the host's
  README.
- **I6.** Deleting an archive record tombstones its vector in the same operation;
  compaction physically removes it.
- **I7.** `model_id`/`dim` mismatch on task submit or on `recall_deep` is rejected
  with an error, never silently accepted.
- **I8.** Vector files are never treated as a source of truth by any code path.

## 11. Calibration of `min_sim` (mandatory before shipping)

Cosine similarities of e5-family models cluster high: unrelated text pairs often score
0.70+. The default `0.75` is a starting point, not a verdict.

Procedure: enable vectors on a test scope with real archive data, collect ~10 real
"пам'ятаєш..." queries, run `recall <scope> "..." --debug`, note the `sim` of the
weakest true positive and the strongest false positive, set `min_sim` between them,
set `vivid_sim` near the strong-match cluster. Record the chosen values and the model
id in `DEVLOG.md`.

## 12. Performance envelope

- Flat scan, 100k rows x 384 dims: ~38M multiply-adds, < 50 ms single-core naive
  Rust; fine without SIMD work.
- RAM: full matrix cached per active scope (~1.5 KB/row). 100k rows ~ 150 MB; typical
  scopes are far smaller. Cache is per-scope, lazily loaded on first recall, and
  LRU-evicted above `cache_max_scopes`, so a host with thousands of scopes pays RAM
  only for the scopes active right now; everything else stays on disk.
- Revisit only past ~500k rows per scope: introduce an HNSW-backed `VectorIndex`
  implementation (`usearch` crate) behind the existing trait. Consider an external
  service (self-hosted Qdrant) only if the engine ever becomes multi-process or
  multi-machine. Neither is in scope now.

## 13. Testing checklist

- f32 file roundtrip: write rows, reopen, byte-exact read-back; R1 truncation
  recovery (simulate crash after vector append, before row append); undersized file
  -> `corrupt`.
- Normalization: unnormalized input vectors are normalized on write and on query.
- Scoring math: unit tests for recency/strength/score with fixed `now`.
- Threshold gating: hit just below `min_sim` excluded even with max recency bonus.
- Token budget cut order (lowest score dropped first).
- Tombstone skipped at search; removed at compaction; events merged into
  `recall_count` / `last_recalled_at`.
- Backfill resumability: kill mid-backfill, restart, cursor continues, no duplicate
  rows (text_hash dedupe).
- Model mismatch rejection (I7), scope isolation (I2), disabled guarantees (I1: assert
  zero embed tasks emitted), purge (I3), rebuild determinism (I4).
- End-to-end with a **fake deterministic embedder** (e.g. seeded hash of text ->
  pseudo-vector) so the test suite needs no model download.

## 14. Implementation order

1. Add this file to `docs/`, link it from README "Main Documents", note the feature
   in `HISTORY.md`.
2. Extend `docs/contracts.md` with: `vectors/` file formats (section 3), the
   `embed_batch` payloads (section 5), the recall result shape (section 8.3).
3. Core: `flat_index` module (open / append / search / tombstone / compact / R1
   recovery, atomic writes, per-scope cache).
4. Core: scope `vector_recall` state + config plumbing + state machine (section 4).
5. Core: sleep stage that emits `embed_batch` for enabled scopes (new records +
   backfill cursor) and the submit handler (validate, normalize, append).
6. Core: `recall_deep` with scoring, threshold, budget, recall events.
7. PyO3 adapter: surface from section 7, including embedder registration and the
   adapter helper that auto-executes `embed_batch` tasks with the registered embedder.
8. Host `telegram_gemini_bot`: fastembed wiring, Gemini tool + prompt addition,
   typing UX, `/memory_vectors` commands.
9. `memory_terminal` commands + calibration pass on real data (section 11), record
   results in `DEVLOG.md`.
10. Tests per section 13 throughout, not at the end.

## 15. Explicitly out of scope (later)

- HNSW / approximate search (`usearch`), f16 or int8 vector quantization.
- Near-duplicate detection at index time (new vector vs existing, `sim > 0.95` ->
  emit a merge-review task instead of silently storing twins). Good v1.1 candidate:
  it mimics humans merging similar memories and fits "promoted through review".
- True reconsolidation (rewriting a memory's text after recall adds new context).
- Drill-down from a recalled memory to its raw source dialogue. Archive records keep
  source references per existing contracts (`TODO(align)`); exposing a host-facing
  "expand this memory" call is a later feature.
- Multi-process file locking, encrypted-at-rest vectors.
- Vector recall over Core layer (Core is small and always in context; it does not
  need semantic search).
