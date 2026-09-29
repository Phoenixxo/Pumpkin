# Phase 01 - Spatial interest index and bounded networking
This phase builds on the measurements from [phase 00](00-baseline-and-benchmarks.md) and describes proposed changes to upstream Pumpkin at `4426d1113`. The [documentation index](../README.md), [system overview](../architecture/overview.md), and [networking design](../architecture/networking.md) provide the broader capacity model and data path.

## Outcome and scope

Pumpkin can avoid repeated world-player scans by maintaining an index of observers for each watched area. The same phase gives incoming and outgoing work explicit memory budgets at both connection and process scope. It must preserve Java and Bedrock wire behavior and the ordering of required packets. Success means less lookup and repeated encoding or compression work under dense fanout, bounded memory under slow readers, and equivalent client-visible state.

The observer interface should remain stable when [phase 03](03-region-ownership.md) introduces region owners and [phase 04](04-ecs-migration.md) changes entity storage. Neither region actors nor ECS are prerequisites here. Cancellable events continue to follow the [plugin design](../architecture/plugin-pipeline.md) and [phase 02](02-plugin-execution.md).

## Source touchpoints

| Current source | Existing behavior to preserve or replace |
|---|---|
| [`World::broadcast_to_chunk` and related methods](../../crates/pumpkin/src/world/mod.rs) | These methods scan world players, test `watched_section`, group Java clients by protocol version, serialize once per Java version, and serialize Bedrock per recipient. The current visibility predicate remains the compatibility reference until a validated subscription lifecycle replaces it. |
| [Java decoder](../../crates/pumpkin-protocol/src/java/packet_decoder.rs) | The decoder validates lengths and returns frozen `Bytes` payloads after optional decryption and decompression. Eligible paths should retain their existing bounds and avoid an extra copy. |
| [Java encoder](../../crates/pumpkin-protocol/src/java/packet_encoder.rs), [client queue](../../crates/pumpkin/src/net/java/mod.rs), and [outgoing writer](../../crates/pumpkin/src/net/java/outgoing.rs) | The writer reuses compression scratch, batches frames, offloads compression through `spawn_blocking`, and encrypts with a connection-owned stream cipher. Normal and priority channels are unbounded, although per-client bytes are accounted for. |
| [Bedrock encoder](../../crates/pumpkin-protocol/src/bedrock/packet_encoder.rs) and [Bedrock queue/writer](../../crates/pumpkin/src/net/bedrock/mod.rs) | Bedrock uses its own framing and compression and guards outbound pending bytes. Its current send path also copies data into the transport session. |
| [Packet limiter](../../crates/pumpkin/src/net/packet_limiter.rs) and [network cap](../../crates/pumpkin/src/net/mod.rs) | The server has a per-client packet-count token bucket and a 64 MiB per-client pending-payload threshold. Neither mechanism imposes a process-wide byte cap. |

## Delivery sequence

### 1. Instrument the old path

Use the fixed workloads and hardware from [phase 00](00-baseline-and-benchmarks.md) to establish a before-and-after comparison. Low-cardinality metrics should cover `broadcast_to_chunk`, serialization by edition, Java frame creation and compression, encryption, socket writes, and ingress decoding. Allocation profiles should include both sparse worlds and the 500-player/3×3-chunk scene. Record the number and age of pending required and replaceable packets alongside RSS.

Keep client versions, compression settings, view distance, chunk churn, entity count, and plugin load constant across comparisons. Measure p99 commit-to-write time alongside p99 tick time: reducing serializer CPU does not help if clients see older state.

### 2. Introduce an observer index behind the current broadcast API

The proposed index maps each dimension and watched section or chunk to stable player IDs and connection generations. A send resolves the current peer from that identity, which prevents a stale subscription from reaching a replacement connection. Watch radius, section, dimension, chunk delivery, and disconnect changes update the index. Batched read-only snapshots give broadcasts a stable view while a short mutation lock or owner task applies changes; fanout should not hold a world-wide lock.

`World::broadcast_to_chunk` can remain the compatibility facade during migration. In shadow mode, the index and current scan both select recipients for test and benchmark workloads, but only one path sends packets. Compare exact recipient IDs and ordering, investigate every mismatch, and switch the live path only when the chosen behavior is understood. A client must receive its spawn or chunk baseline before dependent deltas and must stop receiving updates after despawn. If the current watched-section predicate includes areas whose chunks have not loaded, state and test the chosen compatibility rule with real clients.

A versioned snapshot interface lets the later [spatial owner](../architecture/spatial-ownership-and-ecs.md) find observers without depending on today's `World::players` representation.

### 3. Define wire profiles and share compatible frames

The existing Java serialization-per-version optimization is the starting point. A frame can be shared more widely only when its **bytes and framing** are identical. Compatibility therefore depends on edition, protocol version, negotiated features, packet fields and IDs, recipient-specific entity identifiers, position baseline, compression enabled/threshold/level, and visibility result. Compatible Java recipients can reuse a bounded, short-lived prepared frame. `TCPNetworkEncoder::write_frame` remains the connection-owned encryption boundary: shared preparation never advances another connection's cipher state.

Java's wire contract requires zlib framing, so Zstandard cannot replace it on that path. Bedrock needs its own codec and framing analysis, followed by real-client compatibility checks before frame reuse changes. Codec implementations or SIMD kernels deserve evaluation when profiles show that compression or interest filtering consumes a meaningful share of CPU. The measurements should include cache hits, misses by reason, saved encode and compression work, and frame residency.

### 4. Bound outgoing work and retain ordering

Each unbounded normal or priority queue needs weighted byte admission at both client and process scope. Once an enqueue succeeds, it holds its reservations until send, replacement, cancellation, or disconnect; every exit path releases them exactly once. An atomic compare-exchange loop or weighted semaphore makes the capacity check and reservation one operation, avoiding concurrent overcommit. Memory telemetry must include queue-node overhead and retained shared frames. The host memory envelope should determine cap values, rather than inheriting the current 64 MiB per-client threshold.

Packet families need different policies because some state can be replaced while other state forms the client's protocol history:

| Class | Queue rule | Required check |
|---|---|---|
| Required ordered state | Maintain protocol order and reserve control capacity. Disconnect a persistently slow client within a bounded time when required state cannot drain. | Spawn, despawn, chunk, inventory, and correction packets must not disappear silently. |
| Replaceable state | Retain the newest semantic value by recipient, entity, and field within a freshness budget. Encode against the last **transmitted** baseline or send an absolute resynchronization. | Dropping an intermediate relative update must not corrupt later position or rotation. |

Owner and gameplay paths offer packets without waiting. When a queue is full, they must apply the class policy immediately rather than wait on a Tokio socket, compression task, or peer connection mutex. The writer also needs fairness so priority traffic cannot starve normal state indefinitely.

### 5. Add weighted ingress admission

The existing packet-count token bucket remains useful, but it cannot account for packet size or decompression expansion. Add wire-byte and decoded-byte budgets per connection and globally, and bound owner mailboxes by both items and bytes. Expensive decompression and parsing should consume admission budget before they can monopolize CPU, with protocol-correct rejection or disconnect behavior. When an owner queue fills, the connection pauses reads or rejects work according to a documented policy.

The Java `BytesMut → Bytes` payload path remains. A handler may retain shared backing bytes while its command owns them, but retained receive-buffer bytes need measurement alongside allocation counts. Memory-mapped buffers do not improve the live socket path here.

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
| Interest parity | Exercise watcher movement, view-radius changes, teleports, dimension changes, chunk delivery, and disconnect deterministically. The shadow index must match the intended scan behavior or a documented correction; property tests cover subscription add and remove generations. |
| Java protocol | Test supported versions with compression and encryption on and off, including threshold boundaries. Shared and per-connection frames must be byte-for-byte equivalent or decode to equivalent packets. |
| Bedrock protocol | Run encoder and decoder round trips and real-client smoke tests for negotiated compression, mixed editions, and recipient-specific payloads. |
| Backpressure | Exercise one and many slow readers, priority saturation, and disconnect while a shared frame is queued. RSS and queue bytes must stay within budgets, and required packets must remain ordered or the connection must close. |
| Coalescing | Exercise drop and replacement schedules around spawn, teleport, relative movement, and despawn. Reconstructed client state must reach the latest committed state within the configured age budget. |
| Dense fanout | Exercise 500 players in 3×3 chunks, a 1,000-observer broadcast, and 10,000 mostly idle connections. Record deliveries and bytes per second, CPU by encode/compress/encrypt/write, cache hit rate, RSS, p99 tick, and p99 commit-to-write delay. |
| Sparse world | Exercise distant players and active entities with little shared visibility. Index maintenance and cache overhead must not materially regress p99 latency against phase 00. |

Completion requires protocol and ordering parity, per-client **and global** queue bounds under slow-reader tests, and a measured fanout gain without stale client state. Publish the benchmark configuration and results. [Phase 05](05-capacity-validation.md) evaluates the 10,000-player target, while the [bottleneck matrix](../architecture/bottlenecks.md) tracks risks across pillars.
