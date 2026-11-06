# Cargo dependency licence audit (SAK-196)

Recorded: 2026-11-06
Command: `cargo license` (cargo-license 0.7.0) against `Cargo.lock` / package metadata at branch `sak-196/add-license` (base `d8bbb27` + this change).

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
