# Minimal dual-ABI PoC

Two small Wasm plugin crates and one native host. Each plugin has its own Store;
both Stores share one operation implementation and the same toy health state.
The loader selects the host path from the component's versioned imports.

```text
v0.1 sync guest  -> v0.1 host binding -> shared heal operation
v0.2 async guest -> v0.2 host binding -> shared heal operation
```

## Files

- [guest-v01/src/lib.rs](guest-v01/src/lib.rs): sync guest import and sync `run` export.
- [guest-v02/src/lib.rs](guest-v02/src/lib.rs): async guest import and async `run` export.
- [host/src/main.rs](host/src/main.rs): shared operation, both typed host bindings, version routing and execution.
- [wit/plugin.wit](wit/plugin.wit): tiny worlds selecting each versioned API.
- [wit/deps/v01/api.wit](wit/deps/v01/api.wit): frozen sync API.
- [wit/deps/v02/api.wit](wit/deps/v02/api.wit): async API.
- [run.py](run.py): build and execute the example.

## Verified

Built both guests for `wasm32-unknown-unknown`, encoded and validated both
components, and ran them through Wasmtime 49.0.2. Guest bindings use
wit-bindgen 0.62.0. The native host deliberately yields before modifying health,
so the sync guest waits across a genuine async suspension of its Rust host function.

```text
v0.1 sync guest: health=14
v0.2 async guest: health=20
```

Starting health is 10. The v0.1 guest adds 4; the v0.2 guest adds 6 to that same
state. Runtime assertions verify both outputs. Neither guest binary is rewritten
or composed with an adapter. Generated components are written to `components/`.

## Relation to Pumpkin

This proves sync and async ABIs can coexist with a shared operation. It is a
standalone toy, not a patch to Pumpkin or a full v0.1 compatibility certification.
In particular it does not test callbacks, resource ownership, cancellation,
reentry, opposing roots or Pumpkin's custom scheduler.

The demo's legacy path uses Wasmtime's async Rust host wrapper for a sync
component import. In the actual Pumpkin integration the existing v0.1
`pump_blocking` execution path remains; the v0.2 PR uses its existing
`run_blocking` path. Both can invoke shared game operations. These paths were
checked at commit `83e467f76` of the v0.2 guest SDK PR, where both `heal`
wrappers already reach `player.heal(amount)`.

## Reproduce

Requires Rust with `wasm32-unknown-unknown` installed and the `wasm-tools` CLI.
From the repository root:

```bash
python3 examples/dual-abi-poc/run.py
```

This example is a separate Cargo workspace. The script builds and validates both
components before running the host. Generated components and Cargo build output
are ignored by Git.
