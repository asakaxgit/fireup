# Cargo dependency licence audit (SAK-196)

Recorded: 2026-11-06
Command: `cargo license` (cargo-license 0.7.0) against `Cargo.lock` / package metadata at branch `sak-196/add-license` (base `d8bbb27` + this change).

Since SAK-218 this audit is no longer the only record: [`deny.toml`](../../deny.toml)
encodes the same policy in a form CI enforces on every commit — see
[How this is enforced](#how-this-is-enforced). The prose here remains the *why*;
`deny.toml` is the *what*, and it is the file that fails the build.

## Decision

Project licence: **Apache-2.0**.

Rationale (CTO):

- Explicit patent grant suits a Firebase-compatible local data plane
- Aligns with vendored public `googleapis` protos (Apache-2.0)
- Dependency tree is fully permissive-compatible (see counts below)
- Apache-2.0 §6 already withholds trademark rights; `brand/LICENSE` states the carve-out explicitly

All 230 locked crates use permissive expressions (MIT and/or Apache-2.0,
plus BSD/ISC/Zlib/BSL/Unicode/Unlicense as dual/triple options).
The only LGPL appearance is `r-efi` as `Apache-2.0 OR LGPL-2.1-or-later OR MIT`
(MIT/Apache selectable — no copyleft obligation).
No GPL/AGPL/SSPL/BUSL/proprietary-only crates found.

## Expression counts

```
 154  Apache-2.0 OR MIT
  48  MIT
   7  Apache-2.0
   6  Apache-2.0 OR Apache-2.0 WITH LLVM-exception OR MIT
   3  MIT OR Unlicense
   3  Apache-2.0 OR MIT OR Zlib
   3  (Apache-2.0 OR MIT) AND Unicode-3.0
   2  Apache-2.0 OR BSL-1.0 OR MIT
   2  Apache-2.0 OR BSD-2-Clause OR MIT
   1  BSD-3-Clause AND MIT
   1  Apache-2.0 OR LGPL-2.1-or-later OR MIT

TOTAL 230
```

## Full `cargo license` output

```
(Apache-2.0 OR MIT) AND Unicode-3.0 (3): unicode-ident, unicode-normalization, unicode-properties
Apache-2.0 (7): fireup, prost, prost-build, prost-derive, prost-types, sqlparser, sync_wrapper
Apache-2.0 OR Apache-2.0 WITH LLVM-exception OR MIT (6): linux-raw-sys, rustix, wasi, wasi, wasip2, wit-bindgen
Apache-2.0 OR BSD-2-Clause OR MIT (2): zerocopy, zerocopy-derive
Apache-2.0 OR BSL-1.0 OR MIT (2): wasite, whoami
Apache-2.0 OR LGPL-2.1-or-later OR MIT (1): r-efi
Apache-2.0 OR MIT (154): android_system_properties, anstream, anstyle, anstyle-parse, anstyle-query, anstyle-wincon, anyhow, async-trait, autocfg, base64, base64, bitflags, bitflags, block-buffer, bumpalo, cc, cfg-if, chacha20, chrono, clap, clap_builder, clap_derive, clap_lex, cmov, colorchoice, const-oid, core-foundation-sys, cpufeatures, crc32fast, crossbeam-deque, crossbeam-epoch, crossbeam-utils, crypto-common, ctutils, deadpool, deadpool-postgres, deadpool-runtime, digest, either, equivalent, errno, fallible-iterator, fastrand, find-msvc-tools, fixedbitset, fnv, futures-channel, futures-core, futures-sink, futures-task, futures-util, getrandom, getrandom, hashbrown, hashbrown, hashbrown, heck, hermit-abi, hmac, http, httparse, httpdate, hybrid-array, hyper-timeout, iana-time-zone, iana-time-zone-haiku, indexmap, indexmap, is_terminal_polyfill, itertools, itoa, js-sys, lazy_static, libc, lock_api, log, md-5, mime, multimap, num-traits, num_cpus, once_cell, once_cell_polyfill, parking_lot, parking_lot_core, percent-encoding, petgraph, pin-project, pin-project-internal, pin-project-lite, postgres-protocol, postgres-types, ppv-lite86, prettyplease, proc-macro2, quote, rand, rand, rand_chacha, rand_core, rand_core, rayon, rayon-core, regex, regex-automata, regex-syntax, rustversion, scopeguard, serde, serde_core, serde_derive, serde_json, sha2, shlex, signal-hook-registry, siphasher, smallvec, socket2, socket2, stringprep, syn, syn, tempfile, thiserror, thiserror-impl, thread_local, tokio-io-timeout, tokio-postgres, typenum, unicode-bidi, utf8parse, uuid, wasm-bindgen, wasm-bindgen-macro, wasm-bindgen-macro-support, wasm-bindgen-shared, web-sys, windows-core, windows-implement, windows-interface, windows-link, windows-result, windows-strings, windows-sys, windows-sys, windows-targets, windows_aarch64_gnullvm, windows_aarch64_msvc, windows_i686_gnu, windows_i686_gnullvm, windows_i686_msvc, windows_x86_64_gnu, windows_x86_64_gnullvm, windows_x86_64_msvc
Apache-2.0 OR MIT OR Zlib (3): objc2-core-foundation, objc2-system-configuration, tinyvec
BSD-3-Clause AND MIT (1): matchit
MIT (48): async-stream, async-stream-impl, axum, axum-core, bytes, dashmap, h2, http-body, hyper, libredox, matchers, mio, nu-ansi-term, phf, phf_shared, protoc-bin-vendored, protoc-bin-vendored-linux-aarch_64, protoc-bin-vendored-linux-ppcle_64, protoc-bin-vendored-linux-s390_64, protoc-bin-vendored-linux-x86_32, protoc-bin-vendored-linux-x86_64, protoc-bin-vendored-macos-aarch_64, protoc-bin-vendored-macos-x86_64, protoc-bin-vendored-win32, redox_syscall, sharded-slab, slab, strsim, tokio, tokio-macros, tokio-stream, tokio-test, tokio-util, tonic, tonic-build, tower, tower-layer, tower-service, tracing, tracing-attributes, tracing-core, tracing-log, tracing-serde, tracing-subscriber, try-lock, valuable, want, zmij
MIT OR Unlicense (3): aho-corasick, byteorder, memchr
```

## How this is enforced

Recorded: 2026-11-06 (SAK-218), cargo-deny 0.20.2 against the `Cargo.lock`
committed in SAK-216.

The audit above was a point-in-time snapshot taken by hand. [`deny.toml`](../../deny.toml)
turns it into a gate: the `deny` job in [`ci.yml`](../../.github/workflows/ci.yml)
runs it on every commit, so a dependency carrying a licence outside the allow
list fails the build instead of being noticed at the next manual audit.

```bash
cargo deny --locked check -D advisory-not-detected advisories licenses bans sources
```

### The allow list

Nine SPDX identifiers, which is exactly the set appearing in the current
`Cargo.lock` — nothing speculative:

`Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-2-Clause`, `BSD-3-Clause`,
`BSL-1.0`, `MIT`, `Unicode-3.0`, `Unlicense`, `Zlib`.

Two entries worth knowing about:

- **`BSD-3-Clause` is required, not optional.** `matchit` is
  `BSD-3-Clause AND MIT` — a conjunction, so allowing MIT alone does not
  satisfy it.
- **`LGPL-2.1-or-later` is deliberately absent.** It appears once, as one arm of
  `r-efi`'s `Apache-2.0 OR LGPL-2.1-or-later OR MIT`. Because that expression is
  a disjunction, allowing MIT satisfies it and cargo-deny resolves `r-efi` to
  MIT, so no copyleft obligation enters the tree — the conclusion SAK-196 reached
  by hand, now checked by a tool. Keeping LGPL out of the list means a future
  crate that is *only* LGPL fails.

`ISC` is mentioned in the SAK-196 prose above but carried by no crate in the
current lock file, so it is not in the list either.

`[graph]` sets no `targets` filter. Restricting the graph to Linux would hide
the Windows/macOS/wasi crates that are in `Cargo.lock` and that anyone building
Fireup on those platforms pulls.

### Accepted vulnerability

`RUSTSEC-2026-0258` — `h2` 0.3.27 accepts and queues unbounded empty DATA
frames (low severity). Path: `fireup -> tonic 0.11 -> axum 0.6 -> hyper 0.14 ->
h2 0.3.27`. The fix is `h2 >= 0.4.16`, which needs hyper 1.x, which needs
tonic >= 0.12 and prost 0.13; `Cargo.toml` pins tonic 0.11 on purpose to keep a
single prost version across the data plane and the export parser. Accepted
because the only h2 surface in Fireup is the `fireup local` gRPC server, a
loopback development emulator. Review date 2027-02-06, recorded in `deny.toml`.
The migration is tracked as SAK-220.

Be precise about the limit here: cargo-deny has no expiry mechanism for `ignore`
entries, so that date is a commitment, not a gate. What *is* machine enforced is
that the list cannot silently rot — `-D advisory-not-detected` promotes "this
ignored advisory no longer matches anything" from a warning to an error, so the
entry must be deleted the moment the migration lands.

### `cargo audit` is not used

`cargo deny check advisories` reads the same RustSec advisory database that
`cargo audit` does. Running both would report each vulnerability twice and give
two places to maintain an ignore list. cargo-deny also covers licences,
duplicate crates, and registry provenance in the same binary, so it is the one
kept; `cargo audit` is not installed in the workspace and is not in CI.

### Negative controls

A check that passes proves nothing until you have watched it fail. Each of these
was run against the committed configuration and then reverted:

| Mutation | Result |
|---|---|
| Delete the `RUSTSEC-2026-0258` entry from `[advisories] ignore` | `error[vulnerability]: h2 unbounded empty DATA frames` — `advisories FAILED`, exit 1 |
| Add a `RUSTSEC-2020-0001` entry that matches nothing | `error[advisory-not-detected]` — `advisories FAILED`, exit 1 (a *warning*, exit 0, without `-D advisory-not-detected`) |
| Remove `"MIT"` from `[licenses] allow` | 47 `error[rejected]` diagnostics — `licenses FAILED` |

These are not automated. A Rust test shelling out to `cargo-deny` would either
fail or vacuously skip on a machine without the binary installed, and the thing
under test is a declarative config read by a third-party tool rather than
first-party logic. Re-run the table by hand when `deny.toml` changes materially.
