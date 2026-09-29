# Networking, interest management, and broadcast

Pumpkin already shares serialized Java packets across recipients that use the same protocol version. The next scaling problem is broader: the server must identify interested recipients cheaply, control the amount of work admitted to each connection, and preserve each client's view of authoritative state under load. This chapter describes that design against upstream revision `4426d1113`. Its proposed types and algorithms are not yet implemented. The [system overview](overview.md) places networking in the full architecture; [phase 00](../phases/00-baseline-and-benchmarks.md) defines the workload, and [phase 01](../phases/01-spatial-index-and-networking.md) turns this design into an implementation plan.

## Capacity constraint

A continuous world still has a network fanout limit. Suppose 500 players move while every other player can see them. At 20 updates per second, the server must attempt `500 × 499 × 20 = 4,990,000` recipient deliveries each second. At an illustrative 100 bytes per delivery, those updates alone amount to roughly 499 MB/s, or 4 Gbit/s. Transport headers, compression framing, encryption, chunks, entities, and retransmission add further work. If every mover has 1,000 observers, the count rises to 10 million deliveries per second. These figures define a stress scenario; they are not measurements of Pumpkin.

Shared frames and coalesced movement can remove redundant work, but fixed hardware cannot sustain unlimited all-to-all visibility at a fixed tick rate. Any capacity claim therefore needs a stated view distance, movement rate, edition and protocol mix, entity density, plugin workload, host configuration, and player-visible latency target. The [capacity validation plan](../phases/05-capacity-validation.md) makes that envelope explicit.

## Current behavior upstream

| Path | Observed implementation | Scaling implication |
|---|---|---|
| Java receive | `TCPNetworkDecoder` bounds packet length and processes optional AES-CFB8 decryption and zlib decompression. It fills reusable `BytesMut` scratch and returns a `Bytes` payload through `split_to(...).freeze()`. | The decoder already avoids a final payload copy. Decryption and decompression still transform the data, and typed handlers may allocate while parsing it. |
| Ingress rate | `PacketRateLimiter` enforces a per-client token bucket based on packet count. | Admission should also account for wire and decoded bytes, because a small number of large packets can consume substantial CPU and memory. |
| Java broadcast | `broadcast_java_grouped` serializes once per Java protocol version and gives recipients clones of the same `Bytes` handle. | This sharing should remain. `broadcast_to_chunk` still scans loaded players in the world and checks each player's watched sections. |
| Java send | Each connection has unbounded normal and priority payload channels and closes at a per-client 64 MiB pending-payload threshold. Its writer batches frames, reuses zlib scratch, offloads batches that need compression through `spawn_blocking`, and owns the stream cipher. | Compatible payloads can still incur separate framing and compression. Queue entries consume aggregate memory, while encryption and socket writes remain connection-specific. |
| Bedrock send | `broadcast_bedrock_grouped` serializes separately for each recipient. `BedrockBatchEncoder` allocates intermediate buffers and uses deflate when compression is enabled. The NetherNet send path copies a slice into a new `Bytes`; Bedrock outbound channels are also unbounded with a per-client pending-byte guard. | Framing and transport need edition-specific measurements before optimization. A Java frame cannot be sent as a Bedrock frame. |

Source: [Java decoder](../../crates/pumpkin-protocol/src/java/packet_decoder.rs), [Java encoder](../../crates/pumpkin-protocol/src/java/packet_encoder.rs), [world broadcast](../../crates/pumpkin/src/world/mod.rs), [Java client queue](../../crates/pumpkin/src/net/java/mod.rs), [Java outgoing writer](../../crates/pumpkin/src/net/java/outgoing.rs), [Bedrock client](../../crates/pumpkin/src/net/bedrock/mod.rs), [Bedrock encoder](../../crates/pumpkin-protocol/src/bedrock/packet_encoder.rs), and [packet limiter](../../crates/pumpkin/src/net/packet_limiter.rs).

The 64 MiB threshold applies to **each client**. If 10,000 clients approached that threshold at once, they could account for roughly 625 GiB of pending payloads before all reached their individual limit. Shared frame storage, queue nodes, allocator capacity, and transport buffers make actual resident memory different from that simple sum. A process-wide byte budget is needed alongside the existing client limit.

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

The [spatial owner](spatial-ownership-and-ecs.md) produces a state delta only after gameplay state has committed. The network path then determines which clients need that delta and how to encode it for them. A [plugin decision](plugin-pipeline.md) can reject or transform the underlying action before commit, so a network worker cannot publish speculative state as authoritative.

### Receive without unnecessary copies

Each connection owns a reusable `BytesMut` read buffer and a parser with explicit frame limits. Once the parser has checked length, compression, and encryption state, it freezes the validated payload into `Bytes`. Fields that require no transformation can refer to slices or offsets in that payload. The resulting gameplay command retains the backing `Bytes` until those references are no longer needed; it cannot borrow the connection's scratch buffer across an asynchronous handoff. Fields that need validation or mutation become owned values. The command also carries the connection ID, edition and version, arrival sequence, owner generation, and ingress byte charge.

“Zero copy” means avoiding *additional application copies* on the uncompressed path. Tokio socket reads still move data into userspace; AES and compression change the representation. Memory mapping is appropriate for file-backed caches, not for directly reading a Tokio socket. Decode limits must check compressed length, declared uncompressed length, nesting/collection counts, and work required to parse a packet before admission to the simulation.

Admission has three levels: connection, destination owner, and process. The existing packet-count limiter still protects against packet floods. A second token bucket charges both wire bytes and decoded bytes, while each owner mailbox has limits on item count and retained bytes. When that mailbox fills, the connection task can pause reads. Protocol-specific timeouts and disconnect policy then prevent a stalled peer from holding resources indefinitely.

### Observer index and ownership

An observer index maps watched sections or chunks to stable player IDs. A separate peer directory holds edition and wire-profile metadata. Subscriptions change when a player crosses a section boundary, changes view radius or dimension, or completes chunk delivery. That moves work from every broadcast to the less frequent membership changes. Those changes still need an ordering rule: a new observer needs the entity's spawn and baseline before a relative update, and an observer removed after despawn must not receive a later delta.

An owner publishes immutable, versioned observer snapshots so fanout readers do not contend with subscription updates. It can batch mutations before publication. A lookup may still race a move or disconnect, so the peer writer checks the recipient generation before enqueueing a delivery. Region handoff uses the separate owner generation described in [phase 03](../phases/03-region-ownership.md).

### Wire-compatible frame sharing

Frame sharing is safe only when recipients need identical wire bytes. The cohort key therefore includes edition, protocol version, negotiated features, packet schema, visibility result, recipient-relative baseline, and compression settings. A connection-specific runtime entity ID or relative coordinate baseline divides recipients into separate cohorts even when the gameplay event is the same. Frame-cache entries should live only for the owner revision that produced them; a long-lived global cache would make stale state harder to detect.

For a compatible Java cohort, the broadcaster serializes the raw packet and constructs its standard Java length and compression frame once. Recipients share the resulting immutable `Bytes` value. Bedrock needs its own compatibility key and transport framing; the current encoder constructs a game packet for each recipient. In either edition, the connection writer still owns encryption and the socket write. Java AES-CFB8 ciphertext cannot be shared between connections because each cipher stream has independent state.

The Java wire protocol currently uses zlib. The inspected Bedrock encoder marks a compression method and uses `DeflateEncoder`; any replacement must honor that path's negotiated codec and pass tests with real clients. `zstd-safe` implements Zstandard and is **not** a drop-in compressor for existing Java or Bedrock packets. It is a candidate for internal snapshots or an explicitly negotiated extension. SIMD may help with contiguous distance checks, visibility masks, or a wire-compatible compression implementation when profiles identify those as costly. It does not remove connection-specific encryption or the final socket fanout.

### Bounded egress and semantic coalescing

Egress needs bounded per-client queues and a process-wide byte budget. The accounting charges pending deliveries as well as shared allocations: one frame referenced by 1,000 recipients still represents 1,000 pending writes. A separate residency charge tracks shared frames, preventing a slow final recipient from retaining an unbounded backlog of large buffers. Region owners submit with a nonblocking `try_offer` so a slow socket never holds up the owner tick.

Each outgoing item follows one of two delivery contracts. The distinction determines what the server may do when a queue fills:

| Contract | Examples | Full-queue rule |
|---|---|---|
| Required, ordered | Spawn and despawn, chunk subscription changes, inventory updates, and authoritative corrections require this contract. | The server preserves order and reserves capacity for control traffic. It disconnects a client that cannot make progress by the deadline rather than silently dropping required state. |
| Replaceable state | Position, rotation, cosmetic, and telemetry snapshots may use this contract when a newer value supersedes an older one. | The queue retains the latest semantic value per entity, field, and recipient within a maximum staleness age. The writer encodes any delta against the **last transmitted** baseline when it drains that value. |

An already encoded relative movement packet cannot simply disappear: the next relative packet may depend on it. Coalescing therefore happens before recipient-relative encoding, or the writer sends an absolute resynchronization. Queue service also needs fairness so control traffic does not starve ordinary state indefinitely. Concurrent producers reserve budgets atomically or through a semaphore; checking capacity and incrementing it in separate steps would over-admit.

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

The sketch shows the ownership boundary rather than every lifecycle path. In production, each queue item owns its global and per-client reservations. Sending, replacing, closing, and failure all release those charges. Cipher state remains exclusively with the connection writer.

## Hotspot policy

A 500-player event across 3×3 chunks may behave as one strongly interacting gameplay island. Networking cannot split the simulation dependencies, but it can keep observer lookup and encoding proportional to recipients, cap each client's backlog, and send required transitions first. Replaceable movement can be batched or rate-shaped per observer within an explicit freshness target. Interest changes and entity tracking use the same committed owner epoch, so distant player-less chunks need no broadcast scan or gameplay lock.

Tick rate alone can hide an overloaded network path: positions may be stale even while the owner runs on time. Capacity reports therefore include owner tick latency and the player-visible age of the most recently **transmitted** state. The [bottleneck matrix](bottlenecks.md) treats those as separate signals.

## Verification and observability

The [phase 01 plan](../phases/01-spatial-index-and-networking.md) defines the implementation gates. Its measurements need to cover the full path from decoded input to completed client write:

- Ingress metrics record packets and wire and decoded bytes per second, parsing and decompression CPU, owner-mailbox age, and rejections grouped by cause.
- Interest metrics record candidates per spatial lookup, index update cost, scans avoided, and membership corrections.
- Encoding metrics separate serialization, compression, and encryption CPU by edition and protocol profile, alongside frame-cache hits and retained bytes.
- Egress metrics record deliveries and bytes per second, socket backpressure, per-client and global queued bytes, queue residence time, replacements or drops, and slow-client disconnects.
- Latency metrics report p50, p95, and p99 time from owner commit to completed client write, together with the age of the last transmitted movement state.

Java validation covers compression and encryption in both enabled and disabled configurations, all supported protocol versions, and broadcasts to mixed profiles. Bedrock validation checks codec and framing behavior and mixed-edition visibility with real clients as well as round-trip tests. Load tests include slow readers, 10,000 mostly idle connections, dense and sparse fanout, subscription races, and orderly disconnects. Results are compared with the fixed [phase 00 baseline](../phases/00-baseline-and-benchmarks.md); a serialization microbenchmark alone cannot establish end-to-end capacity.
