# Package and domain names

Where Fireup ships, which names are ours, and which ones are taken. Recorded so
the answers are not rediscovered — and not re-guessed — every release.

Brand and trademark policy is a separate concern; see `TRADEMARKS.md`.

## Decisions

| Registry | Name we ship under | Status |
| --- | --- | --- |
| crates.io | `fireup` | Unclaimed as of 2026-11-08. To be reserved. |
| npm | `@fireup/<pkg>` — **scoped** | Bare `fireup` is taken. Do not plan on it. |
| Domain | pending | `fireup.io` is **not** available; see below. |

### npm ships scoped, permanently

`npm/fireup` is taken by an unrelated package — "Simple process launcher for
local development", latest `2.0.0`, last published 2022-06-18. It is dormant but
dormant is not available, and a dormant package is not transferable on request.

Every JS/TS package we publish therefore uses the `@fireup/` scope:
`@fireup/client`, `@fireup/cli`, and so on. This is a packaging decision, not a
trademark one, and it is not expected to change.

One caveat: whether the `@fireup` **scope itself** is still free is *unverified*.
npmjs.com returns 403 to unauthenticated probes for every scope, including ones
that certainly exist, so the public endpoints cannot answer it. Confirming this
needs a logged-in npm account, and it should be confirmed before the first JS
package is announced — if the scope is taken, the scope name itself has to
change, not just the package name.

### crates.io is the one that matters

Fireup is a Rust binary, so `crates.io/fireup` is the name most likely to be
taken by accident and the one hardest to live without.

Plan of record: publish a **`0.0.0` reservation crate**, not the real `0.1.0`.
The reservation holds the name while the OSS-core / private-cloud split
is still in flight, so that publishing a name does not also publish
source that is still being sorted into public and private halves. The real crate
replaces it when the split has settled and the published tarball is known to
build on its own.

Prerequisites, none of which are satisfied by the current manifest:

- A crates.io API token with publish scope, for the Saks Industries account that
  should own the crate. Read it from the environment; never commit it.
- `license = "Apache-2.0"` — already correct in `Cargo.toml`, and consistent
  with `LICENSE` and the README.
- `repository` and `readme` fields. Both are **absent** from `Cargo.toml` today.
  They are deliberately not added here: the repository URL is exactly what
  the OSS-core / private-cloud split may change, so adding it now would mean
  changing it twice.

A published version cannot be deleted, only yanked, and yanking leaves the
metadata in place. Everything above is checked before the first publish, not
after.

## Verifying availability

A name that was free last week is not free today, so these are re-checked at the
moment of claiming rather than read off this table.

Two traps are worth knowing, because the first one already produced a wrong
answer in the earlier name-clearance check:

1. **RDAP does not cover `.io` or `.sh`.** Neither TLD has an entry in IANA's
   bootstrap registry, so `rdap.org` returns `404` for *every* name under them —
   including `crates.io` and `bun.sh`. A `404` there means "no server for this
   TLD", not "available". This is how `fireup.io` came to be recorded as free.
2. **WHOIS needs a control on the same TLD, in both directions.** Some registry
   servers answer `TLD is not supported` (`.run`, `.tools`, `.systems`) and some
   answer a plain not-found. Before trusting "available", query a name on that
   TLD that is *definitely registered* and confirm the server returns a real
   record. Before trusting "taken", query a nonsense name and confirm it comes
   back not-found.

Checked this way with `whois`, as of **2026-11-08**:

| Name | Status | Evidence |
| --- | --- | --- |
| `crates.io/fireup` | **free** | API `404`, search returns 0 crates |
| `fireup.sh` | **free** | not-found; control `bun.sh` returns a full record |
| `fireup.rs` | **free** | not-registered; control `docs.rs` returns a record |
| `fireup.build` | **free** | not-found; control `nic.build` returns a record |
| `usefireup.com` | **free** | no match; control `fireup.com` returns a record |
| `fireup.io` | **taken** | created 2015-11-25, expires 2027-11-25, Cloudflare |
| `fireup.com` | taken | `clientTransferProhibited`, parked on Afternic |
| `fireup.dev` | taken | active, parked on Afternic |
| `getfireup.com` | taken | created 2021-06-03, GoDaddy |
| `fireup.run`, `fireup.tools`, `fireup.systems` | **unknown** | registry WHOIS answers `TLD is not supported`; needs a registrar lookup |

`fireup.io` serves no site — a CloudFront `403` — and sits behind Cloudflare DNS
with a 2027 expiry. It is held, not used. That makes it a purchase negotiation
rather than a registration, which is why it is not the default choice.

## Open

- **Domain not chosen.** `fireup.io` is unavailable, so the `.io` vs `.sh` choice
  from the earlier name-clearance check no longer exists as stated. Awaiting a
  decision between the verified-free candidates.
- **`@fireup` npm scope unverified.** Needs a logged-in npm account.
- **crates.io reservation not published.** Awaiting a publish token.
