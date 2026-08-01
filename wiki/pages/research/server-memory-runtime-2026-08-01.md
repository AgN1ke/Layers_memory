# Server Memory Runtime: research for Google Play scale

## Related pages

- [Server-scale implementation plan](../planning/server-scale-memory-runtime.md)
  - accepted production track derived from this research.
- [Strategy](../foundation/strategy.md) - project principles this deployment
  shape must preserve.
- [Architecture](../foundation/architecture.md) - current implemented library
  and the accepted server-mode boundary.
- [Contracts](../foundation/contracts.md) - current data contracts and planned
  vault/transaction additions.
- [Roadmap](../planning/roadmap.md) - implementation tasks and gates.

## Purpose and scope

This note answers a concrete product question: what must change when
Chibigochi is distributed through Google Play and thousands of clients use one
central server, rather than each player running one local Godot process with a
private memory directory?

The research compares the current Memory Engine architecture with primary
PostgreSQL and Google documentation and with the implementation shapes used by
Mem0, LangGraph, Graphiti and Letta. It is an architecture study, not a claim
that the server backend already exists.

Date: 2026-08-01.

## Executive conclusion

The Rust cognitive core does not need to be replaced. Its three layers,
pull-based `PendingTask` model, atomic `MemoryUnit`, local embeddings and
provider-independent policy remain the right center of the system.

The current `FileStorage` runtime does need a new production envelope. It is
safe for one writer process per memory directory and its locks exist only
inside that process. A fleet of server replicas cannot safely share it, and a
directory per player would still leave scanning, process coordination,
backups, job ownership and account isolation unsolved.

The accepted shape is therefore a **server-side Memory Runtime** around the
same Rust core:

- one logical `memory vault` per authenticated game account;
- a transactional PostgreSQL storage implementation with `vault_id` on every
  owned record;
- `pgvector` in the same database for the optional derived vector index;
- durable background jobs for embedding, sleep and reflection;
- deterministic human-readable export/import so server operation does not
  sacrifice ownership or auditability;
- `FileStorage` retained as the local, reference and portable-export backend.

The first deployment should keep this runtime as an internal component of the
game backend. Godot calls only the public game API; the backend validates the
external identity and passes an authorized vault context inward. Splitting the
runtime into a private microservice remains an operational option after
metrics, not a prerequisite for supporting thousands of accounts.

This is not “move the JSON files into a database”. The present `Storage` trait
exposes separate reads and writes but no transaction scope. A safe server
backend requires a versioned storage boundary that lets one domain operation
commit atomically.

## What the current code proves — and what it does not

The current in-process `LockRegistry` is useful and tested. Different sessions
can progress in parallel inside one `MemoryEngine`, while conflicting writes
to the same resource serialize. The historical 1,000-session stress run also
shows that independent local session writes do not corrupt each other.

That result is not a production capacity proof:

- locks are not shared by separate processes or server replicas;
- the stress scenario covered ingest/Core writes, not the complete turn with
  recall, vectors, sleep, reflection and provider latency;
- archive, task, journal and vector reads still include flat or full-scope
  scans that become expensive across many players;
- several state transitions are multi-record read-modify-write operations;
- in-memory counters and buffers are replica-local.

The correct conclusion is “the core has a usable concurrency foundation”, not
“the file backend is horizontally scalable”.

## Lessons from comparable GitHub projects

| Project | Useful implementation shape | What we must not confuse it with |
|---|---|---|
| [Mem0](https://github.com/mem0ai/mem0) | Library, self-hosted API and hosted service are separate surfaces; memory calls are namespaced by user/agent/run; the self-hosted stack supports PostgreSQL/pgvector. | A caller-supplied `user_id` is a filter, not authentication. A production server must not accept a client-selected vault or fall back to a shared default user. |
| [LangGraph PostgreSQL checkpointer](https://github.com/langchain-ai/langgraph/blob/main/libs/checkpoint-postgres/langgraph/checkpoint/postgres/__init__.py) | Durable workflow state uses explicit thread/checkpoint namespaces, migrations, upserts, pools and connection-level coordination. | A conversation checkpoint is not the long-term memory model and `thread_id` is not a tenant security boundary. |
| [Graphiti](https://github.com/getzep/graphiti) and its [namespacing model](https://help.getzep.com/graphiti/core-concepts/graph-namespacing) | Every graph node/edge carries a `group_id`; group-filtered search and a separate service boundary make isolation visible. | Namespacing alone does not prove authorization. We do not need Neo4j merely because our hypotheses form a graph. |
| [Letta](https://github.com/letta-ai/letta) and its [multi-user caveat](https://github.com/letta-ai/characterai-memory) | A stateful agent server owns persistent agent state and identity has to be explicit in multi-user deployments. | A local/demo API key or one-worker example is not evidence that a public multi-replica service is safe. |

The common lesson is not “copy one framework”. It is that a stateful agent
needs three separate concepts: authenticated principal, durable memory
namespace, and conversation/workflow namespace. Collapsing them into one
string creates exactly the kind of silent shared-memory failure previously
seen with `local_user`.

## Identity and tenant isolation on Google Play

Google Play Games Services can provide a server authentication code that the
backend exchanges and verifies. Google also states that the platform identity
does not replace the developer's own in-game account and state model. The
client-returned player identifier must not be trusted without server-side
validation.

Therefore the authoritative chain is:

```text
verified Google/guest credential
    -> internal account_id
    -> account-to-vault membership
    -> server-bound VaultStorage
    -> memory records
```

Godot may send a session ID and an idempotency key. It does not send an
authoritative `vault_id`. The runtime derives the vault after authentication
and constructs a storage handle already bound to that vault. This removes the
possibility that one forgotten filter exposes another player's memory.

[Play Integrity](https://developer.android.com/google/play/integrity/standard)
is useful defense in depth for tampering, replay and abusive requests, and can
bind a verdict to a request hash. It is not the account identity system and
must not become the only key to a memory vault.

The [Google Play User Data policy](https://support.google.com/googleplay/android-developer/answer/10144311?hl=en)
also makes the memory lifecycle a launch concern rather than a later admin
feature. Collection/handling must match the privacy policy and Data safety
declaration. If the game creates accounts, deletion must be available in-app
and through an external web resource and must cover associated user data, not
merely freeze the account. Vault deletion/export and documented retention
therefore belong to the production acceptance path.

Database row-level security is a second guard, not the first one. PostgreSQL
[row security policies](https://www.postgresql.org/docs/17/ddl-rowsecurity.html)
can default-deny access when enabled without a matching policy, but table
owners normally bypass RLS. The production runtime role must therefore be a
non-owner, policies must fail closed, and cross-record references should use
composite ownership keys such as `(vault_id, record_id)`.

Connection pooling adds a subtle isolation requirement: no vault may be stored
as session-global connection state and assumed to disappear when a request
ends. A pooled connection can serve another player next. Any RLS context must
be set transaction-locally on every transaction (or supplied explicitly to a
parameterized policy/query) and fail closed when absent.

## Transactions, ordering and background work

Replacing `FileStorage` method bodies with SQL is insufficient. A logical
operation such as sleep completion, reflection apply or candidate review reads
several records, validates their versions and writes several desired records.
Those actions need one storage transaction or an explicit durable state
machine.

The first implementation should serialize short mutations within one vault by
locking a vault row (or by a transaction-scoped advisory lock) while allowing
different vaults to proceed independently. PostgreSQL advisory locks are
application-defined and easy to misuse; ordinary row locks are the simpler
default. If serializable isolation is used, the complete transaction must be
retryable because PostgreSQL may abort it on conflicts.

No database transaction or vault lock may be held while an LLM or embedding
model runs. The existing pull-based task design already gives the correct
shape:

1. validate and commit a durable task;
2. release the transaction;
3. execute provider/local-model work;
4. submit the result with an operation/task id and expected input versions;
5. revalidate and atomically commit the result.

Workers can claim queue rows with PostgreSQL
[`FOR UPDATE SKIP LOCKED`](https://www.postgresql.org/docs/16/sql-select.html),
which is intended for queue-like multi-consumer access. It should not be used
to make ordinary recall reads “faster”, because skipped rows would make those
reads inconsistent.

A separate invariant is required for live chat: two concurrent turns from the
same player must not commit conversational history in the opposite order after
different LLM latencies. Each request needs a server-issued/validated turn
sequence and a client idempotency key. Database atomicity alone does not solve
long-running turn ordering.

## PostgreSQL and vector search

The production tables should be pooled by `vault_id`, not split into one
schema, database or partition per player. PostgreSQL warns that poor partition
choices increase planning and maintenance costs. Fixed hash partitions may be
introduced later from measurements; thousands of user partitions are not the
starting design.

The optional vector index belongs in the same transactional data plane through
[pgvector](https://github.com/pgvector/pgvector):

- filter by `vault_id`, model and eligibility first;
- use exact cosine search while a vault contains a modest number of memory
  units; exact search has perfect recall;
- add HNSW/IVFFlat only after a benchmark demonstrates a need;
- remember that approximate indexes apply filters after scanning candidates
  and may under-return unless their search settings/iterative scans are tuned.

This avoids a second authoritative store and makes account deletion, backup,
model migration and provenance easier. It also preserves the existing rule:
vectors are opt-in, rebuildable search material over `MemoryUnit`, never a
fourth memory layer or the definition of truth.

## Service and database capacity nuances

[Cloud SQL connection guidance](https://docs.cloud.google.com/sql/docs/postgres/manage-connections?hl=en)
emphasizes that connections are limited resources and that a failed connection
inside a transaction requires retrying from the beginning. Pool size is a
system-wide budget: `replica count × pool size` must stay below database limits
with room for workers and administration.

[Cloud Run concurrency](https://docs.cloud.google.com/run/docs/about-concurrency)
can be high by default, but that is only a scheduling setting, not proof that a
stateful application is thread-safe. Begin with deliberately bounded
concurrency, observe CPU, pool wait and vault-lock wait, then tune. Autoscaling
must not turn into a database connection storm.

Initial production should be one region, multiple stateless runtime replicas,
one PostgreSQL primary and durable workers. Active-active multi-region memory
would add ordering and conflict semantics before the product needs them. A
future sharding layer can assign one vault to exactly one database/region.

Local embeddings also change shape at server scale. The model should be loaded
and warmed once per worker process, never per player or request; compatible
jobs can be batched. Readiness must include a model/dimension check so the
first real player does not pay an invisible cold start. A model upgrade writes
a new versioned vector scope and backfills it while structural recall remains
available.

## Preservation of the project spirit

“Files are truth” was the correct operational rule for the local library, but
the deeper project principle is user ownership, inspectability, portability
and recoverability — not a requirement that a public game server coordinate
thousands of mutable directory trees.

In server mode, PostgreSQL is the operational source of truth for a vault.
Every durable record remains versioned and exportable to a deterministic,
human-inspectable bundle that the existing `FileStorage` can read or migrate.
Import/export is part of acceptance, not an optional administrative script.
There is one authoritative backend per vault at a time; transactional outbox
events may build exports or backups, but dual authoritative writes are
forbidden.

This keeps the cognitive architecture intact while changing the deployment
architecture required by the product.

## Alternatives deliberately rejected for the first production version

- Shared `FileStorage` on a network disk: no safe multi-process transaction or
  practical indexed tenant access.
- One directory/database/schema/partition per player: operational explosion
  without meaningful additional safety.
- A separate vector database: unnecessary consistency and deletion burden
  while per-vault exact pgvector search is small.
- Neo4j for the hypothesis graph: current nodes, evidence and relations fit
  relational tables; add a graph engine only if measured traversal workloads
  later require it.
- Redis/Kafka as mandatory infrastructure: PostgreSQL durable jobs and an
  outbox are sufficient for the first scale target.
- Dual-writing files and PostgreSQL as equal truth: partial failure creates two
  divergent memories.
- Client-selected scope with a default fallback: repeats the shared
  `local_user` failure class.
- Holding locks during provider calls: converts variable model latency into
  database contention and outages.

## Measurements required before launch

The existing thread stress test remains useful as a regression, but production
acceptance must exercise whole turns: user ingest, embedding, context/recall,
assistant ingest, and a realistic fraction of sleep/reflection jobs.

The product forecast still has to replace provisional numbers. Until then the
engineering envelope is: 10,000 stored vaults, 1,000 simultaneously active
players, 100 completed turns/second sustained and 300/second burst, with the
acceptance run at twice the expected launch peak. The gate must report p50/p95/
p99 memory-runtime latency, pool wait, vault-lock wait, queue age/depth, vector
latency, retries, cross-vault violations and restore/export results. These are
planning assumptions, not published benchmark claims.

## Primary sources

- PostgreSQL: [row security](https://www.postgresql.org/docs/17/ddl-rowsecurity.html),
  [transaction isolation](https://www.postgresql.org/docs/17/transaction-iso.html),
  [explicit/advisory locks](https://www.postgresql.org/docs/16/explicit-locking.html),
  [`SELECT ... SKIP LOCKED`](https://www.postgresql.org/docs/16/sql-select.html),
  [partitioning](https://www.postgresql.org/docs/current/ddl-partitioning.html).
- Google Play: [server-side access](https://developer.android.com/games/pgs/android/server-access?hl=en),
  [Play Games Services identity overview](https://developer.android.com/games/pgs/overview),
  [Android sign-in security note](https://developer.android.com/games/pgs/android/android-signin?hl=en),
  [Play Integrity](https://developer.android.com/google/play/integrity/standard),
  [User Data policy](https://support.google.com/googleplay/android-developer/answer/10144311?hl=en),
  [account deletion requirements](https://support.google.com/googleplay/android-developer/answer/13327111?hl=en-EN).
- Google Cloud: [Cloud SQL connection management](https://docs.cloud.google.com/sql/docs/postgres/manage-connections?hl=en),
  [managed connection pooling](https://docs.cloud.google.com/sql/docs/postgres/managed-connection-pooling),
  [Cloud Run concurrency](https://docs.cloud.google.com/run/docs/about-concurrency).
- GitHub implementations: [pgvector](https://github.com/pgvector/pgvector),
  [Mem0](https://github.com/mem0ai/mem0),
  [LangGraph PostgreSQL checkpointer](https://github.com/langchain-ai/langgraph/blob/main/libs/checkpoint-postgres/langgraph/checkpoint/postgres/__init__.py),
  [Graphiti](https://github.com/getzep/graphiti),
  [Letta](https://github.com/letta-ai/letta).
