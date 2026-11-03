# Data Generator

TypeScript generators that seed the **Firestore emulator** and export emulator state for Rust functional tests. Exports land in `tests/.firestore-data/` (Firebase emulator export layout with LevelDB-backed `firestore_export/`).

Functional tests that depend on these fixtures are documented in [`../functional-tests/README.md`](../functional-tests/README.md).

## Prerequisites

From `tests/data-generator/`:

```bash
npm ci
```

You also need:

- **Node.js** (compatible with the versions in `package.json`)
- **Java** (required by the Firebase emulator via `firebase-tools`, invoked through `npx`)
- **Firebase CLI** — pulled in as a dev dependency; scripts use `npx firebase …`

Emulator ports are defined in `firebase.json`:

| Service   | Port  |
|-----------|-------|
| Firestore | 18080 |
| Emulator UI | 14000 |

Generators connect using `FIRESTORE_EMULATOR_HOST` (set automatically by `firebase emulators:exec`, or `localhost:18080` for `generate:no-emulator`).

## Sample data ladder (levels 1–4)

Each level has a dedicated npm script: build TypeScript, start the emulator, run the generator, and export on exit.

| Script | Generator | Purpose (summary) |
|--------|-----------|-------------------|
| `npm run generate:level1` | `level1` | Documents with two primitive fields (`level1_test`) |
| `npm run generate:level2` | `level2` | Two documents, same single-field shape (`simple_items`) |
| `npm run generate:level3` | `level3` | Two primitives × two rows |
| `npm run generate:level4` | `level4` | Arrays of primitive values (`level4_arrays`) |

Run all four in sequence:

```bash
npm run generate:all-levels
```

## Other generators

| Script / CLI | Generator | Notes |
|--------------|-----------|--------|
| `npm run generate` | `main` | Users + cities demo dataset |
| `npm run generate:no-emulator -- <name>` | any | Emulator must already be running (see below) |
| `node dist/run-generator.js <name>` | see below | After `npm run build` |

`run-generator.js` accepts: `level1`, `level2`, `level3`, `level4`, `main`, `tiny` (see `src/run-generator.ts`).

## Typical workflows

### One-shot generate + export (recommended)

Uses `firebase emulators:exec` so the emulator starts, the generator runs, and data is exported when the command finishes:

```bash
cd tests/data-generator
npm run generate:level2
```

Export directory: `tests/.firestore-data/` (`--export-on-exit=../.firestore-data` in the npm scripts).

Equivalent after a manual build:

```bash
npm run build
npx firebase emulators:exec --export-on-exit=../.firestore-data 'node dist/run-generator.js level2'
```

### Long-running emulator (development)

**Terminal 1** — start emulator, import existing export, export again on shutdown:

```bash
npm run dev
# or: npm run emulator
# or: npm run emulator-ui   # same as emulator (UI enabled in firebase.json)
```

**Terminal 2** — generate while the emulator is up:

```bash
npm run generate:no-emulator -- level2
# Examples: level1, level3, level4, main, tiny
```

`generate:no-emulator` runs `npm run build`, sets `FIRESTORE_EMULATOR_HOST=localhost:18080`, then `node dist/run-generator.js` with any arguments you pass after `--`.

### Build only

```bash
npm run build
```

Compiles `src/` to `dist/` (`tsc`).

## Viewing data in the emulator UI

With the emulator running:

- Open http://localhost:14000 (Firestore tab)
- Optional helper (macOS): `npm run open-ui`

For `main`, expect collections such as `users` and `cities`; ladder levels use the collection names described in each generator under `src/`.

## How fixtures reach Rust tests

1. Generators write documents to the Firestore emulator.
2. `firebase emulators:exec` / `--export-on-exit` writes an emulator export under `tests/.firestore-data/`.
3. Functional tests read that tree (for example `tests/.firestore-data/firestore_export/…`). See [`../functional-tests/README.md`](../functional-tests/README.md) for `cargo test` invocations.

Level 1/3 tests may skip when fixtures are missing; level 2/4 follow the same pattern when data is absent.

## Adding a new generator

1. Add `src/my_generator.ts` (follow existing files: require `FIRESTORE_EMULATOR_HOST`, use `firebase-admin`).
2. Register the name in `src/run-generator.ts` (`generatorMap`).
3. `npm run build`
4. Run via `npm run generate:no-emulator -- myname` (emulator running) or add an npm script that wraps `firebase emulators:exec` like the `generate:level*` scripts.

## Troubleshooting

- **`FIRESTORE_EMULATOR_HOST environment variable is required`** — start the emulator (`npm run dev`) or use a `generate:level*` / `generate` script that uses `emulators:exec`.
- **Empty UI** — confirm http://localhost:14000 and that Firestore is on port 18080 (`firebase.json`).
- **Stale export** — stop the emulator cleanly so `--export-on-exit` can write `../.firestore-data`; or delete `tests/.firestore-data` and regenerate.
- **Permission / Java errors from Firebase** — install a JRE/JDK and retry `npx firebase emulators:exec …`.
