# Phase 02 - Bounded plugin execution

This phase applies the [plugin pipeline](../architecture/plugin-pipeline.md) to the workload defined in [phase 00](00-baseline-and-benchmarks.md). It follows the network bounds in [phase 01](01-spatial-index-and-networking.md) and establishes an invocation contract that [phase 03](03-region-ownership.md) can route to region owners.

## Outcome

The design must preserve v0.1 cancellation and returned-event behavior while bounding the effect of guest CPU, host calls, and queued work on core workers. An invocation should own its request and reply so that later execution domains can route it to a world, region, or entity. Upstream at `4426d1113` does not yet provide Region execution or an async v0.2 WIT host.

## Baseline to retain

The [plugin manager](../../crates/pumpkin/src/plugin/mod.rs) currently awaits both mutable and immutable handler groups, and `fire_blocking` bridges synchronous callers into that path. The [WASM loader](../../crates/pumpkin/src/plugin/loader/wasm/mod.rs) attaches a `LegacySyncReentry` gate to its loader; the [Store executor](../../crates/pumpkin-plugin-runtime/src/executor.rs) already has bounded channels and reentry support. [Movement](../../crates/pumpkin/src/net/java/play/player_position.rs) and [block placement](../../crates/pumpkin/src/block/registry.rs) require decisions before gameplay can commit. Upstream's [`pumpkin-scheduler` crate](../../crates/pumpkin-scheduler/src/domain.rs) documents Global admission but is not wired into `pumpkin`. It is a useful starting point, not a live region or plugin scheduler.

## Implementation sequence

The sequence first makes existing event semantics explicit, then introduces a single admission rule and owned invocation boundary. Resource limits and the new ABI follow once that boundary can reject, time out, and discard work safely.

### 2.1 Inventory event semantics and host calls

An inventory of native and WASM entrypoints, event categories, synchronous bridges, and host imports establishes which calls need a decision before commit and which merely observe committed state. For each event, record mutation and cancellation behavior, permitted coalescing, and required ordering. The inventory must cover commands, lifecycle hooks, scheduled tasks, and nested calls as well as packet events. It should also identify host APIs that can block, perform I/O, mutate world state, or reenter dispatch.

### 2.2 Add one causal admission boundary for legacy semantics

Operations that still require legacy global order need one fair, chain-aware admission rule above the entire plugin manager, including native and WASM handlers. Extend and integrate upstream's [Global scheduler](../../crates/pumpkin-scheduler/src/global.rs) for that role. Nested calls reuse the causal chain ID; independent roots queue fairly. This **transitional compatibility policy** shares one ordering boundary rather than adding a competing scheduler. During migration, retain the Store reentry mechanism and one outer synchronous bridge. A second Tokio runtime or recursive `block_on` would create another blocking path. Measure gate occupancy and queue age, especially under high-frequency movement.

### 2.3 Define owned invocation and decision replies

Each invocation should carry an `InvocationId`, causal chain, origin domain, target generation, immutable event snapshot, deadline, and fail policy. Decision requests enter a bounded queue while the caller parks its staged transaction. On reply, the caller revalidates the event or source version and owner generation before committing. A timeout produces an explicit failure result and, where necessary, a client correction; a late reply cannot commit. Observation events use a separate bounded queue after commit.

Until [phase 03](03-region-ownership.md) establishes owner tokens, the current world path can stand in for a future Region domain. Parallel region mutation is not available at this stage. A same-region host call needs a staged overlay or another explicit read-your-writes rule; otherwise an event can wait for a request to the owner whose transaction it has parked.

### 2.4 Add WASM and host budgets

Configure fuel and/or epoch yielding or trapping in the pinned Wasmtime revision, and prove with a tight-loop test that the configured limit takes effect. Wall-clock deadlines, Store resource limits, per-plugin event and host-call queue limits, host-call count and byte budgets, and per-import timeouts bound work that fuel cannot see. Charge host work separately from guest instructions. Bounded executor capacity keeps guest execution off Tokio I/O reactor workers and Rayon simulation workers. A memory limiter alone cannot provide this isolation.

Repeated plugin failures should trip a circuit breaker with diagnostics. Unload and shutdown must either drain or reject queued and in-flight calls according to a defined lifecycle policy. WASM fuel cannot preempt a native callback, so trusted native plugins need cooperative async behavior, process isolation, or a documented weaker guarantee.

### 2.5 Specify and integrate an async v0.2 ABI without redefining v0.1

Specify async WIT callbacks and host imports as an upstream-facing contract, then implement negotiation and the new host boundary. Existing [v0.1 WIT](../../crates/pumpkin-plugin-wit/v0.1/plugin.wit) behavior remains available until plugins opt in. Domain-aware host calls carry owned inputs and replies. One logical task owns a Store unless a plugin explicitly supports partitioned state. A compact typed event snapshot usually avoids copying a large network payload into WIT; socket-to-WASM zero-copy is not part of this contract.

## Validation gate

The gate tests semantic compatibility, bounded failure, and worker isolation together. A plugin timeout is meaningful only if a late reply cannot still alter authoritative state.

| Test | Required result |
| --- | --- |
| v0.1 parity | Native and WASM handler order, returned event changes, cancellation, and client corrections must match the declared legacy contract. |
| Causal reentry | Same-region requests, cross-domain calls, opposing plugin roots, and `A → host → B → host → A` must finish or fail by deadline without deadlock. |
| Malicious guest | Infinite loops, excessive memory growth, and host-call floods must yield, trap, or be rejected within configured limits. Queues and core-worker occupancy must remain bounded. |
| Decision timeout | Placement and movement must follow their documented failure policies. A late result cannot commit state or leak a pending transaction. |
| Lifecycle | Unload, reload, shutdown, and cancellation must drain or reject pending work deterministically. |
| Performance | Record p99 plugin decision and host-call queue age and Global-gate occupancy. A stalled guest must not delay unrelated Tokio I/O or Rayon simulation beyond the phase 00 reference budget. |

Once these gates pass, the [region-ownership phase](03-region-ownership.md) can route host commands to spatial owners. Cancellable plugin semantics still include a bounded decision wait; the contract promises bounded isolation and explicit timeout behavior, rather than zero gameplay latency.
