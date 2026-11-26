# Production-recorded fidelity oracle

Board decision: approve with constraints (pull request #139). Compliance rule:
[`.cursor/rules/firebase-gcp-compliance.mdc`](../../.cursor/rules/firebase-gcp-compliance.mdc).

## What it is

Fireup may run a small set of **doc-cited** scenarios against a dedicated
Spark-plan Firebase/GCP sandbox. The run checks whether production does what a
**public Google doc already says**. Fixtures store pass/fail verdicts on those
assertions only. CI **never** talks to Google; it replays the committed fixtures
against `fireup local`.

The Firebase Emulator Suite is **never** an oracle.

## Constraints (binding)

1. No public doc citation → no case (`catalog.json` enforces this).
2. Fixtures store verdict fields only — never raw bodies, tokens, timings.
3. Production diverges from the doc → `verdict: "unsupported"`; never copy production.
4. Scope: Firestore Native, Datastore mode, Auth v1 email/password only.
5. Synthetic data; `example.com` emails; delete created data after each run.
6. Credentials: Paperclip secrets only; CI holds none.
7. Record on demand or via the monthly Paperclip routine — never per PR.
8. Ledger rows + checklist column when the recorder lands.

## Sandbox projects

| Project id | Mode |
|---|---|
| `fireup-oracle-firestore` | Firestore Native + Auth (Spark, no billing) |
| `fireup-oracle` (env override; docs historically said `fireup-oracle-datastore`) | Datastore mode (Spark, no billing) |

Service account roles: `roles/datastore.user` + `roles/firebaseauth.admin`.
Web API key restricted to Identity Toolkit + Secure Token APIs.

## Record (operator / monthly routine)

```bash
# Secrets arrive via Paperclip — never paste into comments or the repo.
export GOOGLE_APPLICATION_CREDENTIALS="$PAPERCLIP_ORACLE_FIRESTORE_SA_PATH"
export FIREUP_ORACLE_FIRESTORE_CREDENTIALS="$PAPERCLIP_ORACLE_FIRESTORE_SA_PATH"
export FIREUP_ORACLE_DATASTORE_CREDENTIALS="$PAPERCLIP_ORACLE_DATASTORE_SA_PATH"
export FIREUP_ORACLE_WEB_API_KEY="$PAPERCLIP_ORACLE_WEB_API_KEY"
export FIREUP_ORACLE_DATASTORE_PROJECT=fireup-oracle
scripts/recorded-oracle-record.sh
```

The recorder refuses `FIRESTORE_EMULATOR_HOST` / `DATASTORE_EMULATOR_HOST` /
`FIREBASE_AUTH_EMULATOR_HOST` and refuses to run under `CI` / `GITHUB_ACTIONS`.

If production disagrees with the cited sentence, the fixture is marked
`unsupported` and an issue should be filed. Do **not** change Fireup to match
undocumented production behaviour.

## Replay (CI)

`scripts/fidelity-smoke-gate.sh` ends with `scripts/recorded-oracle-replay.sh`,
which starts `fireup local` and compares assertion maps to
`tests/recorded-oracle/fixtures/`.

Unit checks for the catalog and fixture schema run as
`npm test --prefix tests/recorded-oracle` (also from the fidelity job).

## Bootstrap fixtures

Fixtures may ship with `productionVerified: false` until the board finishes
creating the Spark projects and proposing Paperclip secrets. The first
successful `recorded-oracle-record.sh` overwrites them with
`productionVerified: true`. Offline replay still exercises `fireup local`
against the doc-expected assertion maps in the meantime.

## On-demand path

Any operator with the Paperclip secrets can re-record without waiting for the
monthly routine: run `scripts/recorded-oracle-record.sh`, review the fixture
diff (verdicts only), and open a PR. Never commit credential files.
