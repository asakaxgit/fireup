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

One identifier has since drifted out of that "nothing speculative" claim:
`cargo deny` now reports `warning[license-not-encountered]` for
`BSD-2-Clause`, which no crate in the lock file carries any more — the tonic
0.11 → 0.13 migration dropped whatever did. It is a warning rather than an
error, and removing an identifier from `allow` is a policy change rather than a
cleanup (a crate carrying it would start failing), so SAK-225 left it in place
and recorded it here instead of deciding it in passing.

`[graph]` sets no `targets` filter. Restricting the graph to Linux would hide
the Windows/macOS/wasi crates that are in `Cargo.lock` and that anyone building
Fireup on those platforms pulls.

### Unmaintained advisories: the whole graph, not just direct dependencies

`[advisories] unmaintained = "all"`.

SAK-218 wrote `"workspace"` instead, with a reason that sounded right:
"all" would make the job red for an abandoned crate buried four levels down
that we have no way to replace. SAK-225 measured that setting against the tree
and found it hiding one real advisory on the day it was written:

| | |
|---|---|
| Advisory | `RUSTSEC-2024-0449` — `unsafe-libyaml` is unmaintained |
| Crate | `unsafe-libyaml` 0.2.11 — libyaml's C, machine-translated by c2rust |
| Why unmaintained | repository archived read-only 2024-03-25; 0.2.11 is the last release; no patched version exists |
| Path | `fireup -> (dev) serde_yaml 0.9.34+deprecated -> unsafe-libyaml 0.2.11` |
| Reported under `"workspace"` | no — the crate is transitive, so it fell outside the scope |
| Reported under `"all"` | yes — `advisories FAILED`, exit 1 |

The premise that failed was "we have no way to replace it". We could not
replace `unsafe-libyaml`, but we never chose it: we chose `serde_yaml`, for one
test that reads this repository's own workflow files. Replacing *that* with
`saphyr` (maintained, YAML 1.2, safe Rust) removed the crate from the tree
altogether, which is why this section is a scope decision and not an entry in
the `ignore` list below. A transitive unmaintained advisory is a question about
one of our own direct dependencies, and that is something we can act on.

`RUSTSEC-2023-0075`, the other `unsafe-libyaml` advisory (unsound unaligned
writes on 16/32-bit targets), was patched in 0.2.10 and never applied to the
0.2.11 the tree carried.

The two settings could not simply be swapped in one commit, and the reason is
worth recording because it will recur: `deny.toml` lives in a stack of PRs that
has not reached `main`, while `serde_yaml` was added to `main` after that stack
branched. No single tree contained both. Raising the scope *and* accepting the
advisory with an `ignore` entry — the shape SAK-218 used for h2 — was therefore
not available: in the only tree that has `deny.toml` the advisory does not
exist, and `-D advisory-not-detected` fails an entry that matches nothing.
Removing `serde_yaml` on `main` and raising the scope in the stack is green in
both trees separately and after they meet.

`tests/dependency_pin_tests.rs` asserts the scope is still `"all"`. cargo-deny
cannot check this itself — the setting is its own input — and narrowing it makes
CI *greener*, so no run would ever report the regression.

### Accepted vulnerabilities: none

`[advisories] ignore` is empty. Every advisory the RustSec database reports
fails the build.

It was not always empty. SAK-218 accepted one entry with a review date of
2027-02-06: `RUSTSEC-2026-0258`, `h2` 0.3.27 accepting and queueing unbounded
empty DATA frames (low severity), reached as
`fireup -> tonic 0.11 -> axum 0.6 -> hyper 0.14 -> h2 0.3.27`. The fix needed
`h2 >= 0.4.16`, so hyper 1.x, so tonic >= 0.12 and prost 0.13 — and `Cargo.toml`
pinned tonic 0.11 on purpose, to keep one prost version across the data plane and
the export parser. SAK-220 did that migration (tonic 0.13 / prost 0.13, which
pulls hyper 1.12 and h2 0.4.20, still one prost version) and deleted the entry in
the same change.

That pairing was not a matter of remembering. cargo-deny has no expiry mechanism
for `ignore` entries, so the review date was a commitment rather than a gate —
but `-D advisory-not-detected` promotes "this ignored advisory no longer matches
anything" from a warning to an error, so the migration could not land while the
entry was still there. An accepted vulnerability here cannot outlive its fix.

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
| `cargo update -p h2 --precise 0.4.15` — put the tree back one patch below the fix | `error[vulnerability]: h2 unbounded empty DATA frames` / `RUSTSEC-2026-0258` — `advisories FAILED`, exit 101 |
| Add a `RUSTSEC-2020-0001` entry that matches nothing | `error[advisory-not-detected]` — `advisories FAILED`, exit 1 (a *warning*, exit 0, without `-D advisory-not-detected`) |
| Remove `"MIT"` from `[licenses] allow` | 47 `error[rejected]` diagnostics — `licenses FAILED` |

SAK-225 added four more, because the `unmaintained` scope is a claim about
crates that are not in this tree yet and a passing run says nothing about it.
Each adds a dev dependency to `Cargo.toml` and re-resolves before running:

| Mutation | Result |
|---|---|
| The committed config, unmodified | `advisories ok`, exit 0 |
| Add `serde_yaml = "0.9"` — the dependency `main` carries today, which this stack will merge into | `error[unmaintained]: unsafe-libyaml is unmaintained` / `RUSTSEC-2024-0449` — `advisories FAILED`, exit 1 |
| The same tree with `unmaintained = "workspace"` restored | `advisories ok`, exit 0 — **the blind spot itself, reproduced** |
| Add `saphyr = "0.3"` — the replacement parser, i.e. `main` after SAK-225 | `advisories ok`, exit 0 |
| Add an `ignore` entry for `RUSTSEC-2024-0449` to this tree | `error[advisory-not-detected]`: *no crate matched advisory criteria* — exit 1 |

The third row is the one to keep: the same tree, the same advisory, the same
command, green or red depending on one word of configuration. That is what a
gate's scope is worth measuring.

These are not automated. A Rust test shelling out to `cargo-deny` would either
fail or vacuously skip on a machine without the binary installed, and the thing
under test is a declarative config read by a third-party tool rather than
first-party logic. Re-run the table by hand when `deny.toml` changes materially.

What *is* automated is the narrower question of whether the lock file and the
config still have the properties these decisions were about.
`tests/dependency_pin_tests.rs` reads `Cargo.lock` and `deny.toml` directly — no
network, no advisory database, no third-party binary — and fails if `h2` drops
below 0.4.16, if `prost` resolves to more than one version, if
`RUSTSEC-2026-0258` reappears in `deny.toml`, or if `unmaintained` is narrowed
back from `"all"`. It is a cheap local echo of the `cargo deny` job, not a
replacement for it.
