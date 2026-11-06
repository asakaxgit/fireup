# Fireup marketing site

A static site. Three files you edit — `index.html`, `styles.css`,
`site.webmanifest` — plus `assets/`, which is **generated** by `brand/` and
should never be hand-edited.

No build step, no framework, no bundler, no runtime dependency. Serving the
directory is the deploy.

## Preview it

```bash
cd site
npx --yes http-server . -p 4173     # or any static server
```

## Regenerate the assets

```bash
cd ../brand && npm run build
```

That rewrites `site/assets/` — logo variants, lockups, favicons, app icons, the
social card, and `tokens.css` — from the masters in `brand/logo/` and
`brand/tokens/brand.json`. `styles.css` uses only those tokens for colour, type,
spacing and radii; a raw hex in the stylesheet is a bug.

## Verify it

```bash
cd site
npm install --no-save puppeteer
npx puppeteer browsers install chrome
npm run shoot
```

`npm run shoot` loads the page in a real browser at 1280×900 and 390×844, and
fails if anything 404s, if the console logs an error, or if an element overflows
the viewport outside a deliberately scrollable box. Screenshots land in
`.shots/` (not committed) — look at them; the exit code only catches what a
machine can see.

## Copy rules

The project is pre-release and the site says so, in the hero, in a dedicated
status section, and in the fidelity matrix. Keep it that way. Every number on
the page traces to a verification run recorded in `PROJECT_STATUS.md` or
`docs/local/README.md`; if one of those changes, change the page. Claiming more
than the tool does is the one failure mode this site cannot recover from.

The footer disclaimer — independent implementation, not the Firebase Emulator
Suite, not affiliated with Google — is required, not decorative. See
`docs/licenses/FIREBASE_GCP_LOCAL.md`.
