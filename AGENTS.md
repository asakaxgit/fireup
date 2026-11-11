# Fireup — agent project memory

Persistent rules for agents working in this repository.

## Compliance (always)

Follow `.cursor/rules/firebase-gcp-compliance.mdc` and keep `docs/licenses/FIREBASE_GCP_LOCAL.md` updated:

- Do **not** reverse-engineer Firebase / Google proprietary emulators or closed binaries.
- Implement against **public** API docs and public `googleapis` protos only.
- Record every Google/Firebase-related dependency and proto pin in the license ledger with correct SPDX/attribution.
- Do not claim official Firebase/Google affiliation.

Origin: project owner direction, recorded in #44.

## This repository is public; the issue tracker is not

Refer to work by its **pull request number** — `#117` — in anything that lands
in the tree: code comments, documentation, commit messages, branch names, pull
request titles and bodies.

Do not write an internal ticket id, and do not link to the internal tracker.
Both are invisible outside the company: the id names nothing a reader can look
up, and the link resolves to nothing. A pull request is the same change's public
identity, and it carries the diff, the review and the checks with it.

Where a reference has no pull request — work that predates them, landed straight
on `main`, or is still only planned — drop it. These sentences describe a
measurement that stands on its own; the id was never what made them true.

A branch gets a short descriptive slug: `drop-internal-tracker-references`, not
a ticket number. The link between a ticket and its change runs tracker → pull
request, which is the direction that works from both sides.

Enforced by `tests/public_repo_hygiene_tests.rs` over every tracked file, so
this is a build failure rather than a convention. Two test files are exempt
while an open pull request stack rewrites them; that exemption is itself
asserted, and has to be deleted once they are swept.
