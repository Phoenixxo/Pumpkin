# Phase 02 — Bounded plugin execution

**Status:** implementation plan. [Phase 00](00-baseline-and-benchmarks.md) defines the benchmark; [phase 01](01-spatial-index-and-networking.md) bounds network work; [phase 03](03-region-ownership.md) later adds real region owners. The architectural contract is in the [plugin pipeline](../architecture/plugin-pipeline.md).

## Outcome

Preserve v0.1 cancellation and returned-event behavior while preventing WASM guest CPU, host calls, or queues from indefinitely occupying core workers. Create an owned invocation and reply boundary that can later route to World, Region, and Entity domains. Upstream at `4426d1113` has neither Region execution nor an async v0.2 WIT host.

## Baseline to retain

The [plugin manager](../../crates/pumpkin/src/plugin/mod.rs) currently awaits both mutable and immutable handler groups. `fire_blocking` bridges from synchronous callers. The [WASM loader](../../crates/pumpkin/src/plugin/loader/wasm/mod.rs) instantiates a `LegacySyncReentry` gate attached to that loader, and the [Store executor](../../crates/pumpkin-plugin-runtime/src/executor.rs) has bounded channels and reentry support. Current [movement](../../crates/pumpkin/src/net/java/play/player_position.rs) and [block placement](../../crates/pumpkin/src/block/registry.rs) are decision events. Upstream's [`pumpkin-scheduler` crate](../../crates/pumpkin-scheduler/src/domain.rs) documents only Global admission and is not wired into `pumpkin`; it supplies an upstream starting point rather than a live region or plugin scheduler.

## Delivery packages

### 2.1 Inventory event semantics and host calls

List every native and WASM entrypoint, event handler category, synchronous bridge, and host import. Mark events as decision or observation, their mutation/cancellation behavior, allowed coalescing, and required ordering. Include command, lifecycle, scheduled-task, and nested plugin calls; a policy that covers only packet events is incomplete. Record which host APIs can block, use I/O, mutate world state, or reenter plugin dispatch.

### 2.2 Add one causal admission boundary for legacy semantics

Integrate and extend upstream's [Global scheduler](../../crates/pumpkin-scheduler/src/global.rs) as a fair chain-aware admission rule above the entire plugin manager, including native and WASM handlers, for operations still requiring one legacy global order. Nested calls reuse their chain ID; opposing roots queue fairly. This is a **transitional compatibility policy**, not a second competing scheduler. Keep the existing Store reentry mechanism and one outer synchronous bridge while callers are migrated; do not introduce a second Tokio runtime or recursive `block_on`. Instrument queue age and occupancy because the gate can limit throughput under high-frequency movement.

### 2.3 Define owned invocation and decision replies

Introduce `InvocationId`, causal chain, origin domain, target generation, immutable event snapshot, deadline, and fail policy. Submit decisions through a bounded queue. The caller stages its transaction and receives an owned result; on return it revalidates event/source version and owner generation before commit. A timeout produces an explicit fail result, client correction where needed, and rejection of later replies. Observation events publish after commit through a separate bounded queue.

The current world path may initially stand in for an unimplemented Region domain. Do not imply parallel region mutation until [phase 03](03-region-ownership.md) establishes owner tokens. Same-region host calls require a staged overlay or other explicit read-your-writes rule; without it, an event that awaits a request to its own parked owner can deadlock.

### 2.4 Add WASM and host budgets

Configure fuel and/or epoch yielding/trapping in the pinned Wasmtime revision, with tests that prove a tight loop yields or traps within its budget. Set wall-clock deadlines, Store resource limits, per-plugin event and host-call queue limits, host-call count and byte budgets, and per-import timeouts. Charge host work separately from guest fuel. Keep guest execution off Tokio I/O reactor workers and off Rayon simulation workers, using bounded executor capacity. The current memory limiter alone is insufficient.

Use a circuit breaker with diagnostics for repeatedly failing plugins. Define unload and shutdown behavior for queued and in-flight calls. Native callbacks cannot be preempted by WASM fuel; require cooperative async behavior, a separately isolated process, or clearly state the weaker guarantee for trusted native plugins.

### 2.5 Specify and integrate an async v0.2 ABI without redefining v0.1

Specify the async WIT callbacks and host imports as an upstream-facing contract, then add negotiation and a host implementation behind the new ABI. Preserve the existing [v0.1 WIT](../../crates/pumpkin-plugin-wit/v0.1/plugin.wit) behavior until plugins opt in. Domain-aware host calls carry owned inputs and replies, and the Store remains owned by one logical task unless a plugin declares and implements partitioned state. Avoid copying large network payloads into WIT events when a compact typed snapshot suffices; no end-to-end socket-to-WASM zero-copy claim is made.

## Validation gate

| Test | Required result |
| --- | --- |
| v0.1 parity | Native and WASM handler order, returned event changes, cancellation, and client corrections match the declared legacy contract. |
| Causal reentry | Same-region request, cross-domain call, opposing plugin roots, and `A → host → B → host → A` finish or fail by deadline without deadlock. |
| Malicious guest | Infinite loop, excessive memory growth, and host-call flood yield/trap or are rejected within configured limits; no unbounded queue or core-worker occupation. |
| Decision timeout | Placement/movement follow their documented fail policies; a late result cannot commit state or leak a pending transaction. |
| Lifecycle | Unload, reload, shutdown, and cancellation drain or reject pending work deterministically. |
| Performance | p99 plugin decision and host-call queue age are recorded. A stalled guest does not delay unrelated Tokio I/O or Rayon simulation beyond the phase 00 reference budget. Global-gate occupancy is visible. |

Only after these gates pass should the [region-ownership phase](03-region-ownership.md) route host commands to real spatial owners. A bounded decision wait remains part of cancellable plugin semantics; the guarantee is bounded isolation, not zero gameplay latency.
