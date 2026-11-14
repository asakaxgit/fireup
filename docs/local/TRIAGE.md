# Fidelity triage role (pilot: Datastore)

Three agent roles split fidelity work so each task stays small enough to
finish:

- **Test-writer:** adds checks of documented behaviour to a backlog suite,
  whether or not Fireup passes them.
- **Triager:** runs the backlog, groups the failures by root cause, and files
  one issue per group.
- **Fixer:** takes one issue, makes its named checks pass, and moves them under
  the [fidelity regression gate](README.md#fidelity-regression-gate).

This page defines the triager. The pilot covers the Datastore surface only.

## Input

```bash
FIREUP_SMOKE_POSTGRES_URL=postgresql://…  # a database you own; see PROJECT_STATUS.md
scripts/triage-datastore-backlog.sh [results-dir]
```

The command runs
[`tests/local-smoke-datastore/backlog.mjs`](../../tests/local-smoke-datastore/backlog.mjs)
against the in-memory store, and against PostgreSQL when the URL is set. It
then prints `tests/smoke-gate/triage.mjs`'s first pass. The results files have
the gate's `{ suite, checks: [{ name, ok }] }` shape. A failed check also
carries `error: { code, message }`. The gate ignores that field, and the
triager groups on it.

The backlog is the suite `datastore-backlog/<store>`. `baseline.json` has no
entry for it, so a red backlog can never fail CI. That is deliberate: a
backlog check exists to fail until someone fixes it.

## First pass: signatures (mechanical)

`triage.mjs` clusters the failures by signature:

- **A refusal** is a check that failed with a gRPC status. Its signature is the
  status name plus the message, with run-specific values blanked.
- **A wrong result** is a check where the server answered, and the answer was
  wrong. It forms its own cluster, because nothing in the result says why the
  answer was wrong.
- **Store-specific:** a check that fails on only some of the stores it ran on is
  flagged. That points at a store bug rather than a surface bug.

## Second pass: root-cause groups (judgement)

A signature is not a root cause. The triager reads the handler code for every
cluster before filing anything:

1. **Merge** clusters that one code path explains. In the first cycle, a
   `__key__` filter and a `__key__` order were two clusters with one cause: the
   handler treats `__key__` as an ordinary property.
2. **Split** a cluster whose checks need separate fixes in separate places.
3. **Classify** every group as one of three kinds:

   | Class | Meaning | Disposition |
   |---|---|---|
   | `bug` | A silent wrong answer, or a divergence from behaviour that a public Google source documents | File a fixer issue now and assign it |
   | `gap` | An explicit, documented `UNIMPLEMENTED` refusal of a production feature | File it in `backlog`, unassigned, for prioritisation |
   | `by-design` | Fireup refuses on purpose, and the README's known limits say so | No issue. Record it in the cycle report |

   A wrong answer is always `bug`, even when the README lists the feature as
   unsupported. A refusal is the documented contract, and an empty result set
   is not one.
4. **Cite a public source** for every `bug` that rests on documented
   behaviour. If no public source states the behaviour, the group becomes
   `by-design` until one does
   ([compliance rule](../../.cursor/rules/firebase-gcp-compliance.mdc)).
5. **Order** groups that edit the same function with `blockedByIssueIds`, so
   fixers do not open parallel PRs against one hunk.

## Fixer issue template

Each issue must make sense to an agent that has read nothing else:

```markdown
**Root cause:** <one sentence: what the code does wrong, and where (file and function)>

**Class:** bug | gap. **Surface:** Datastore. **Triage cycle:** <link to the cycle report>

## Failing checks (from `tests/local-smoke-datastore/backlog.mjs`)
- `<exact check name>`: <observed result> (memory, postgres)

## Repro
scripts/triage-datastore-backlog.sh   # or: FIREUP_SMOKE_ENTRY=backlog.mjs scripts/smoke-local-datastore.sh

## Public source
<URL or vendored proto path that defines the expected behaviour>

## Acceptance
1. Each named check passes on memory and postgres.
2. Each named check moves from `backlog.mjs` into `smoke.mjs`, unchanged apart
   from fixture setup, and `node tests/smoke-gate/gate.mjs --update <results-dir>`
   adds it to `baseline.json`.
3. The `fidelity smoke gate (official SDKs)` check is green on the PR, with no
   other baseline check removed.
4. `docs/local/README.md` and `tests/conformance/matrix.json` say what is now
   served.
5. Reuse existing crates and the shared query/value code; no new infrastructure.
```

## Cycle report

At the end of each cycle the triager records:

- the groups, with their class and the checks in each;
- the issues filed, fixed, and rejected at the gate;
- false groupings, i.e. a fixer found two causes, or two issues turned out to
  be one;
- the agent cost of the cycle.
