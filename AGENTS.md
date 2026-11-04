# Fireup — agent project memory

Persistent rules for agents working in this repository.

## Compliance (always)

Follow `.cursor/rules/firebase-gcp-compliance.mdc` and keep `docs/licenses/FIREBASE_GCP_LOCAL.md` updated:

- Do **not** reverse-engineer Firebase / Google proprietary emulators or closed binaries.
- Implement against **public** API docs and public `googleapis` protos only.
- Record every Google/Firebase-related dependency and proto pin in the license ledger with correct SPDX/attribution.
- Do not claim official Firebase/Google affiliation.

Origin: board direction on Paperclip issue SAK-144.
