# Phase 01 — Spatial interest index and bounded networking

**Status:** implementation plan; no changes described here are implemented by this document. The plan targets Pumpkin upstream at `4426d1113`. Read the [documentation index](../README.md), [system overview](../architecture/overview.md), and [networking design](../architecture/networking.md) for the capacity model and data path. This phase follows [phase 00: baseline and benchmarks](00-baseline-and-benchmarks.md).

## Outcome and scope

Replace repeated world-player scans with an observer index and make ingress and egress memory bounded at both connection and process scope. Preserve existing Java/Bedrock wire behavior and required packet ordering. The intended measurable result is less lookup and repeated encoding/compression work under dense fanout, bounded memory under slow readers, and no regression in client-visible correctness.

This phase does not require region actors or ECS. It should expose stable owner and observer interfaces that [phase 03](03-region-ownership.md) and [phase 04](04-ecs-migration.md) can adopt. Cancellable events remain governed by the [plugin design](../architecture/plugin-pipeline.md) and [phase 02](02-plugin-execution.md).

## Source touchpoints

| Current source | Existing behavior to preserve or replace |
|---|---|
| [`World::broadcast_to_chunk` and related methods](../../crates/pumpkin/src/world/mod.rs) | Scan world players, test `watched_section`, group Java clients by protocol version, serialize once per Java version, serialize Bedrock per recipient. Preserve the current visibility predicate until a validated subscription lifecycle replaces it. |
| [Java decoder](../../crates/pumpkin-protocol/src/java/packet_decoder.rs) | Validates lengths and returns frozen `Bytes` payloads after optional decrypt/decompress. Keep existing bounds and zero-extra-copy behavior on eligible paths. |
| [Java encoder](../../crates/pumpkin-protocol/src/java/packet_encoder.rs), [client queue](../../crates/pumpkin/src/net/java/mod.rs), and [outgoing writer](../../crates/pumpkin/src/net/java/outgoing.rs) | Reuses compression scratch, batches frames, offloads compressing batches through `spawn_blocking`, encrypts with a connection-owned stream cipher, and has normal/priority unbounded channels with per-client byte accounting. |
| [Bedrock encoder](../../crates/pumpkin-protocol/src/bedrock/packet_encoder.rs) and [Bedrock queue/writer](../../crates/pumpkin/src/net/bedrock/mod.rs) | Uses edition-specific packet framing/compression and an outbound pending-byte guard; its current send path includes a copy into the transport session. |
| [Packet limiter](../../crates/pumpkin/src/net/packet_limiter.rs) and [network cap](../../crates/pumpkin/src/net/mod.rs) | A per-client packet-count token bucket and a 64 MiB per-client pending-payload threshold exist. Neither supplies a process-wide byte cap. |

## Delivery sequence

### 1. Instrument the old path

Use the fixed workloads and hardware in [phase 00](00-baseline-and-benchmarks.md). Add low-cardinality metrics around `broadcast_to_chunk`, per-edition serialization, Java frame creation/compression, encryption, socket writes, and ingress decode. Capture allocation profiles for both sparse and 500-player/3×3-chunk scenes. Establish count and age of pending required and replaceable packets, plus RSS.

For benchmark validity, report the same client versions, compression settings, view distance, chunk churn, entity count, and plugin load before and after the changes. Measure p99 commit-to-write time as well as p99 tick time. A faster serializer with older client-visible state is not a successful outcome.

### 2. Introduce an observer index behind the current broadcast API

Create an index keyed by dimension and watched section or chunk. Store stable player IDs and a connection generation; resolve a live peer only at send time. Apply changes when watch radius, section, dimension, chunk delivery, or disconnect changes. Publish read-only snapshots in batches, using a short mutation lock or owner task rather than holding a world-wide lock throughout fanout.

Keep `World::broadcast_to_chunk` signatures as compatibility facades during the migration. Run indexed and scanned recipient selection together in shadow mode on test and benchmark workloads; compare exact recipient IDs and ordering, without double-sending. Only switch the live path after mismatches are explained. The index must ensure a client gets its spawn/chunk baseline before dependent deltas and does not receive updates after despawn. Where the old watched-section predicate overapproximates actual chunk load, choose the compatibility rule deliberately and test it with clients.

The index should expose a versioned snapshot interface to the [spatial ownership design](../architecture/spatial-ownership-and-ecs.md), so a later region owner can ask for observers without knowing today’s `World::players` representation.

### 3. Define wire profiles and share compatible frames

Keep the existing Java serialization-per-version optimization. Group further only when the resulting **bytes and framing** are identical: edition, protocol version, negotiated features, packet fields and IDs, recipient-specific entity identifiers, position baseline, compression enabled/threshold/level, and visibility result. Add bounded, short-lived frame reuse for compatible Java recipients. The current `TCPNetworkEncoder::write_frame` can remain the connection-owned encryption boundary; frame preparation must not share or advance another connection's cipher state.

Do not replace the Java zlib frame with Zstandard. The Bedrock path gets a separate codec and framing analysis, with real-client compatibility before any frame-reuse change. Benchmark codec implementations or SIMD kernels only when profiles identify compression or interest filtering as a meaningful share of CPU. Track cache hits, misses by reason, saved encode/compression work, and frame residency.

### 4. Bound outgoing work and retain ordering

Replace or wrap each unbounded normal/priority queue with weighted per-client and process-wide byte admission. A successful enqueue owns reservations until send, replacement, cancellation, or disconnect; every exit path releases them exactly once. Use an atomic compare-exchange loop or weighted semaphore to avoid an overcommit race between checking capacity and incrementing it. Include queue-node overhead and shared-frame residency in memory telemetry. Derive cap values from the host memory envelope rather than from the current per-client 64 MiB threshold.

Classify packet families:

| Class | Queue rule | Required check |
|---|---|---|
| Required ordered state | Maintain protocol order and reserve control capacity. Apply a time-bounded slow-client disconnect policy when required state cannot drain. | No silently missing spawn, despawn, chunk, inventory, or correction. |
| Replaceable state | Retain the newest semantic value by recipient/entity/field under a freshness budget. Encode against the last **transmitted** baseline, or send an absolute resynchronization. | Dropping an intermediate relative update cannot corrupt later position or rotation. |

Owner and gameplay paths use nonblocking offer. Queue-full handling cannot wait on a Tokio socket, a compression task, or a peer's connection mutex. Enforce fairness so high-priority traffic does not indefinitely starve normal state.

### 5. Add weighted ingress admission

Retain the packet-count token bucket, then add wire-byte and decoded-byte budgets per connection and globally. Limit owner-mailbox items and bytes. Charge expensive decompression/parse paths before they can monopolize CPU, subject to protocol-correct rejection and disconnect behavior. A full owner queue pauses connection reads or rejects according to a documented policy; it does not grow unbounded.

The Java `BytesMut → Bytes` payload path remains, and packet handlers may retain shared backing bytes only while their command owns it. Measure retained receive-buffer bytes as well as allocation counts. Memory-mapped buffers are not part of the socket path.

## Correctness invariants

1. A given semantic update produces the same recipient set as the selected compatibility rule, across movement, view-distance changes, dimension travel, chunk loading, and disconnect.
2. A recipient sees required state transitions in protocol order. A spawn or chunk baseline precedes dependent deltas; a despawn precedes any subsequent state for a reused entity ID.
3. Encryption state belongs to one Java connection writer. Shared frames contain only compatible plaintext framed bytes; ciphertext is never broadcast.
4. The byte counters include every pending delivery and release on every success, replacement, close, and error path. Process-wide pending bytes have a configured hard ceiling.
5. Replaceable coalescing is semantic. The next emitted delta uses the last transmitted baseline or an absolute state, so a skipped packet cannot corrupt position.
6. A slow or malicious client cannot make the world tick await its queue or retain unbounded decode/egress memory.

## Test and benchmark gates

| Gate | Workload and evidence |
|---|---|
| Interest parity | Deterministic watcher movement, view-radius changes, teleports/dimension changes, chunk delivery and disconnect; shadow index equals intended scan behavior or documented corrected behavior. Property tests cover subscription add/remove generations. |
| Java protocol | Supported versions with compression off/on and encryption off/on, including threshold boundaries; byte-for-byte or decoded-packet equivalence for shared versus per-connection frames. |
| Bedrock protocol | Encoder/decoder round trips and real-client smoke tests for the negotiated compression path, mixed editions, and recipient-specific payloads. |
| Backpressure | One slow reader, many slow readers, priority saturation, and disconnect during a queued shared frame. RSS and queue bytes remain within configured budgets; required packets are ordered or the connection closes. |
| Coalescing | Drop/replacement schedules around spawn, teleport, relative movement, and despawn. Reconstructed client state equals the latest committed state within the configured age budget. |
| Dense fanout | 500 players in 3×3 chunks, plus a 1,000-observer broadcast and 10,000 mostly idle connections. Record recipient deliveries/s, bytes/s, CPU by encode/compress/encrypt/write, cache hit rate, RSS, p99 tick, and p99 commit-to-write delay. |
| Sparse world | Distant players and active entities with low shared visibility. Index maintenance and cache overhead must not regress p99 latency materially against phase 00. |

The phase is complete when protocol and ordering gates pass, per-client **and global** queue bounds hold under the slow-reader tests, and the fixed benchmark shows the measured fanout gain without hiding stale client state. Publish the benchmark configuration and results; the 10,000-player target itself is evaluated in [phase 05](05-capacity-validation.md). The cross-pillar risk register is in the [bottleneck matrix](../architecture/bottlenecks.md).
