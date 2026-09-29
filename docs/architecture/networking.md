# Networking, interest management, and broadcast

**Status:** architecture proposal. Source observations refer to Pumpkin upstream at `4426d1113`; the proposed structures and pseudocode are not implemented. Start at the [documentation index](../README.md) or the [system overview](overview.md). The delivery plan is [phase 01](../phases/01-spatial-index-and-networking.md); the workload definition belongs to [phase 00](../phases/00-baseline-and-benchmarks.md).

## Capacity constraint

A continuous world does not remove network fanout. If 500 players all move and each of the other 499 can observe them, a 20 Hz stream produces `500 × 499 × 20 = 4,990,000` recipient deliveries per second. At an illustrative 100 bytes per delivery, that is roughly 499 MB/s, or 4 Gbit/s, **before** transport headers, compression framing, encryption overhead, chunks, entities, and retransmission. With 1,000 observers per mover the count is 10 million deliveries per second. These are workload calculations, not Pumpkin benchmarks.

An implementation can avoid redundant serialization and reduce replaceable updates, but it cannot promise 20 TPS for unlimited all-to-all visibility on fixed hardware. The acceptance envelope must state view distance, movement rate, editions and versions, entity density, plugin load, host, and player-visible latency. See [capacity validation](../phases/05-capacity-validation.md).

## Current behavior upstream

| Path | Observed implementation | Scaling implication |
|---|---|---|
| Java receive | `TCPNetworkDecoder` bounds packet length, reads through optional AES-CFB8 decryption and zlib decompression, fills reusable `BytesMut` scratch, then `split_to(...).freeze()` returns a `Bytes` payload. | This already avoids a final payload copy. Decryption and decompression still transform bytes, and typed packet handlers may allocate. |
| Ingress rate | `PacketRateLimiter` uses a per-client packet-count token bucket. | Add byte and decompressed-byte budgets; a few large packets can consume more CPU and memory than many small ones. |
| Java broadcast | `broadcast_java_grouped` serializes once per Java protocol version and clones a shared `Bytes` handle for recipients. | Preserve this optimization. Current `broadcast_to_chunk` scans the loaded world-player vector and filters each player's watched section. |
| Java send | Each connection queues payloads in normal and priority unbounded channels, with a per-client 64 MiB pending-payload close threshold. The writer batches frames, reuses zlib scratch, and offloads batches needing compression with `spawn_blocking`. Each writer owns its stream cipher. | The remaining duplicate work is per-connection framing/compression of compatible payloads, queue metadata and aggregate memory, and per-connection encryption and socket writes. |
| Bedrock send | `broadcast_bedrock_grouped` serializes separately per recipient. `BedrockBatchEncoder` allocates intermediate buffers and uses a deflate encoder when compression is enabled; the NetherNet send path copies a slice into a new `Bytes`. Bedrock also has unbounded outbound channels and the per-client pending-byte guard. | Optimize only after measuring edition-specific framing and transport constraints. Java frames cannot be reused as Bedrock frames. |

Source: [Java decoder](../../crates/pumpkin-protocol/src/java/packet_decoder.rs), [Java encoder](../../crates/pumpkin-protocol/src/java/packet_encoder.rs), [world broadcast](../../crates/pumpkin/src/world/mod.rs), [Java client queue](../../crates/pumpkin/src/net/java/mod.rs), [Java outgoing writer](../../crates/pumpkin/src/net/java/outgoing.rs), [Bedrock client](../../crates/pumpkin/src/net/bedrock/mod.rs), [Bedrock encoder](../../crates/pumpkin-protocol/src/bedrock/packet_encoder.rs), and [packet limiter](../../crates/pumpkin/src/net/packet_limiter.rs).

The 64 MiB threshold is **per client**, not a server memory cap. Ten thousand clients each approaching it would allow approximately 625 GiB of counted pending payloads before all hit their individual threshold. Shared frame storage, allocator capacity, queue nodes, and transport buffers complicate the actual RSS in either direction. Aggregate admission must be measured and bounded separately.

## Proposed data path

~~~mermaid
flowchart LR
    Socket["Java TCP or Bedrock transport<br/>Tokio connection task"] --> Decode["Bounded frame decode<br/>decrypt and decompress as needed"]
    Decode --> Ingress["Packet, byte, and CPU admission"]
    Ingress --> Owner["Spatial owner mailbox<br/>ordered gameplay command"]
    Owner --> Delta["Semantic state delta"]
    Delta --> Interest["Section observer index"]
    Interest --> Cohort["Wire-compatible cohorts"]
    Cohort --> Frame["Serialize/frame/compress once<br/>where legal"]
    Frame --> Egress["Global and per-client<br/>weighted byte admission"]
    Egress --> Writer["Connection-owned encryption<br/>and socket write"]
~~~

The [spatial ownership design](spatial-ownership-and-ecs.md) defines who produces a committed state delta. This document defines how it reaches clients. The [plugin pipeline](plugin-pipeline.md) may reject or transform a gameplay decision before the delta exists; a network worker must not send speculative authoritative state as committed.

### Receive without unnecessary copies

Keep a connection-owned `BytesMut` read buffer and a bounded frame parser. After length, compression, and encryption checks, freeze the validated payload to `Bytes` and carry slices or offsets for fields that can remain borrowed. A gameplay command must own or retain the backing `Bytes` for as long as any slice is used; it cannot borrow the connection scratch across an asynchronous owner handoff. Decode fields into owned values when validation or mutation requires it. Carry connection ID, edition/version, arrival sequence, owner generation, and an ingress byte charge into the owner mailbox.

“Zero copy” means avoiding *additional application copies* on the uncompressed path. Tokio socket reads still move data into userspace; AES and compression change the representation. Memory mapping is appropriate for file-backed caches, not for directly reading a Tokio socket. Decode limits must check compressed length, declared uncompressed length, nesting/collection counts, and work required to parse a packet before admission to the simulation.

Use weighted limits at three levels: per connection, per destination owner, and process-wide. The existing packet-count limiter stays useful. A second token bucket charges wire bytes and decoded bytes; an owner mailbox has a bounded count and byte capacity. The connection task may pause reads when its owner queue is full, while protocol-specific timeouts and disconnect policy prevent a permanently stalled peer from occupying memory.

### Observer index and ownership

Maintain an index from watched sections/chunks to stable player IDs, with edition and wire-profile metadata in a separate peer directory. Update subscriptions when a player changes section, view radius, dimension, or chunk-delivery state, rather than scanning every world player on every broadcast. Membership changes must be sequenced with spawn, chunk load, and despawn. A newly added observer must not receive a relative entity update before its spawn/baseline; a removed observer must not receive a later delta after despawn.

An owner can publish an immutable, versioned observer snapshot for concurrent read-heavy fanout. Mutation and publication are batched. If a lookup races a player move or disconnect, the peer writer validates a recipient generation before enqueue. Region handoff uses the owner generation described in [region ownership phase 03](../phases/03-region-ownership.md).

### Wire-compatible frame sharing

The cache key must describe **actual wire identity**, not merely the same semantic event. It includes edition, protocol version, negotiated features, packet ID/schema, visibility result, recipient-relative baseline, compression enabled/threshold/level, and any other payload-affecting setting. Values tied to a connection, such as a runtime entity ID or relative coordinate baseline, split cohorts. Batch IDs must be stable only within their valid owner revision; a short-lived frame cache is safer than a global cache that risks stale state.

For a compatible Java cohort, serialize one raw packet, construct one standard Java length/compression frame, and share an immutable `Bytes` frame among recipients. For Bedrock, first establish the correct per-edition compatibility key and transport framing; its current encoder constructs a game packet per recipient. The connection writer still performs its own stateful encryption and socket write. Java AES-CFB8 cannot reuse ciphertext across connections because each cipher stream has independent state.

Java's current wire compression is zlib. The inspected Bedrock encoder marks a compression method and uses `DeflateEncoder`; retain the codec negotiated by that Bedrock path and verify against real clients before replacing its encoder. `zstd-safe` is Zstandard, so it is **not** a drop-in Java or Bedrock packet compressor. It can be benchmarked for internal snapshots or an explicitly negotiated extension. SIMD is a candidate for contiguous interest-distance checks, visibility masks, or a wire-compatible compression backend after profiles show those kernels matter; it does not bypass encryption or socket fanout.

### Bounded egress and semantic coalescing

Use bounded per-client queues and a process-wide weighted byte budget. Charge the sum of pending delivery bytes, not just the number of `Bytes` allocations: 1,000 recipients still represent 1,000 pending writes. Track shared frame residency separately so a slow final recipient cannot pin an unbounded number of large frames. Admission from a region owner is a nonblocking `try_offer`; waiting for a slow client must never occupy the owner tick.

Each outgoing item has one of two delivery contracts:

| Contract | Examples | Full-queue rule |
|---|---|---|
| Required, ordered | spawn/despawn, chunk subscription changes, inventory and authoritative corrections | Preserve order. Reserve capacity for control traffic; if the connection cannot make progress within its deadline, disconnect it rather than silently losing required state. |
| Replaceable state | newer position, rotation, cosmetic or telemetry snapshot | Keep the latest semantic value per entity/field/recipient, subject to a maximum staleness age. Encode its delta against the **last transmitted** baseline when the writer drains it. |

Do not drop an already encoded relative movement packet and assume the next relative packet remains valid. Coalescing occurs before recipient-relative encoding or forces an absolute resynchronization. Fairness prevents priority traffic from starving ordinary state indefinitely. Budget reservations must be atomic or semaphore-based; a check followed by an add is insufficient under concurrent producers.

~~~rust
// Design sketch: types and policies, not an implementation against current APIs.
struct WireProfile {
    edition: Edition,
    version: ProtocolVersion,
    features: FeatureBits,
    compression: CompressionProfile,
    payload_baseline: BaselineClass,
}

enum Delivery {
    Required { order: u64, frame: bytes::Bytes },
    Replaceable { key: ReplicationKey, state: StateSnapshot },
}

struct PeerEgress {
    generation: u64,
    queue: tokio::sync::mpsc::Sender<Delivery>,
    pending_bytes: AtomicUsize,
}

fn broadcast_delta(delta: &StateDelta, index: &ObserverSnapshot, peers: &PeerDirectory) {
    for cohort in index.wire_compatible_cohorts(delta, peers) {
        let frame = prepare_compatible_frame(delta, &cohort.profile);
        for peer in cohort.recipients {
            // Atomic weighted reservation is performed before queue insertion.
            // No await and no gameplay lock is held by this path.
            peer.try_offer(frame.clone(), delta.delivery_contract());
        }
    }
}
~~~

This sketch omits lifecycle and errors deliberately. Production code must couple reservations to queue item ownership so both global and per-client charges are released on send, replacement, close, and failure. Connection writers are the sole owners of cipher state.

## Hotspot policy

A 500-player event in 3×3 chunks may form one strongly interacting gameplay island. The network layer does not assume it can split that simulation. It instead lowers lookup and encoding cost, caps per-client backlog, and emits required transitions first. Replaceable movement can be batched or rate-shaped per observer under an explicit freshness target. Interest changes and entity tracking are based on the same committed owner epoch; distant, player-less chunks require neither a broadcast scan nor a lock.

The main risk is hiding lag by delivering stale positions while TPS looks healthy. Report both owner tick latency and player-visible age of the most recently **transmitted** state. This distinction also appears in the [bottleneck matrix](bottlenecks.md).

## Verification and observability

The [phase 01 plan](../phases/01-spatial-index-and-networking.md) owns implementation gates. At minimum, measure:

- Ingress packets/s, wire bytes/s, decoded bytes/s, parse/decompression CPU, owner-mailbox age and rejection by reason.
- Candidate observers per spatial lookup, index update cost, scan avoidance, and membership corrections.
- Encode/compress/encrypt CPU by edition and protocol profile; frame-cache hit ratio and frame bytes retained.
- Deliveries/s, payload and wire bytes/s, socket backpressure, per-client and global queued bytes, queue residence time, dropped/replaced state count, and slow-client disconnects.
- p50/p95/p99 time from owner commit to client write completion, plus the age of the last transmitted movement state.

Validate Java compression on/off, encryption on/off, supported protocol versions, and mixed-profile broadcasts. Validate Bedrock codec/framing and mixed-edition visibility against real clients as well as round-trip tests. Test slow readers, 10,000 mostly idle connections, dense and sparse fanout, subscription races, and orderly disconnects. Compare each result with the fixed [phase 00 baseline](../phases/00-baseline-and-benchmarks.md); do not infer end-to-end capacity from serialization microbenchmarks alone.
