# Recorded fidelity oracle

Doc-cited pass/fail verdicts recorded against a Spark-plan Firebase/GCP sandbox,
replayed offline against `fireup local`. See
[`docs/local/RECORDED_ORACLE.md`](../../docs/local/RECORDED_ORACLE.md) and the
board decision recorded in pull request #139.

## Layout

| Path | Role |
|---|---|
| `catalog.json` | Cases with public doc URL + sentence. No citation → no case. |
| `fixtures/*.json` | Verdict fixtures only (never raw responses, tokens, timings). |
| `record.mjs` | Production recorder (Paperclip secrets; never CI). |
| `replay.mjs` | Offline replay against `fireup local`. |
| `scenarios.mjs` | Shared assertion extractors for record and replay. |

## Commands

```bash
# Offline (CI / local):
scripts/recorded-oracle-replay.sh

# On demand / monthly (operator machine with Paperclip secrets):
export GOOGLE_APPLICATION_CREDENTIALS=/path/to/sa.json
export FIREUP_ORACLE_WEB_API_KEY=...   # from Paperclip secret; never commit
scripts/recorded-oracle-record.sh
```

Bootstrap fixtures ship with `productionVerified: false` until the first sandbox
recording overwrites them. A divergence from the cited doc becomes
`verdict: "unsupported"` — Fireup never copies production.
