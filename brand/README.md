# Fireup brand guide

**Status: proposed, changes requested.** The mark, wordmark and palette in this
directory are a first direction for SAK-173. They are in use on the marketing
site in `site/`, but they are **not approved** — the CEO has asked for changes
to the identity, and those are still to be specified. Identity direction is the
CEO's sign-off.

This directory is in the public repository so the direction can be reviewed in a
pull request. That is not approval. Until the mark is signed off, do not use it
as the company's mark anywhere outside this repo — no published site, no
external announcement, no profile or avatar.

---

## The mark

An **F** whose middle arm has come off the stem and moved to the right.

The stack on the left is the store. The ember row is the one that has been
lifted out and moved somewhere else — which is the whole product in one glyph,
whether that is an export landing in PostgreSQL or a document being served from
a local store. It is a monogram, so it says the name at sizes where a picture
cannot.

### What it deliberately is not

- **Not a flame.** Fireup is a Firebase-adjacent tool, so a flame would read as
  derived from Firebase's own mark; separately, flame logos in software are
  actively enforced trademarks (Tinder has sent infringement notices over a
  flame icon). Nothing here is worth that.
- **Not a power symbol.** An earlier direction — a rounded square with a bar
  breaking out of the top — rendered as the IEC 5009 power glyph. It read as a
  button rather than a brand, and a universal standard symbol cannot be owned.
  It is kept in `scripts/explore.mjs` with that note.

## Files

| File | What it is |
|---|---|
| `logo/mark.svg` | **Master.** The mark, 64-unit grid. |
| `logo/wordmark.svg` | **Master.** "fireup", drawn as paths, not set in a font. |
| `logo/favicon-16.svg` | **Master.** Its own drawing, cut to a 16×16 pixel grid. |
| `logo/favicon-32.svg` | **Master.** Its own drawing, cut to a 32×32 pixel grid. |
| `tokens/brand.json` | **Master.** Colour, type scale, spacing, radii. |
| `tokens/brand.css` | Generated. CSS custom properties, same values. |
| `explore/` | The rendered directions that were considered, including the rejected ones. |
| `tune/` | One sheet per tuning axis, for choosing between variants of the chosen direction. |
| `../site/assets/` | Generated. Every variant, lockup, favicon, app icon and the social card. |

Masters are hand-authored and are the only files to edit. Everything else is
output: **do not hand-edit a generated SVG, PNG or CSS file** — the next build
will overwrite it.

## Rebuilding everything

```bash
cd brand
npm install        # @resvg/resvg-js, the raster pipeline
npm run check      # smoke test the pipeline, verify contrast, rebuild all assets
```

`npm run check` is `smoke` + `contrast` + `build`:

- `npm run smoke` renders a known SVG and asserts the PNG really is the
  requested size with the expected pixels. It is there because "the export
  script ran without erroring" is not evidence that it rendered anything.
- `npm run contrast` computes every palette pair the brand uses and fails if one
  misses its WCAG AA threshold. Nothing is eyeballed.
- `npm run build` regenerates `tokens/brand.css` and all of `site/assets/`.
- `npm run explore` re-renders the exploration sheets.
- `npm run tune` re-renders the four tuning sheets in `tune/`.

`explore` and `tune` are deliberately outside `check`: they are the record of a
decision, not shipped assets, and they should only change when the question
being asked changes.

## Choosing between variants

`npm run tune` renders the four axes that can move without redrawing the
direction — the ember colour, the gap between the lifted arm and the stem, the
height the arm sits at, and the wordmark weight. Each sheet puts three options
side by side at the sizes they will really be used at, and for the mark it also
magnifies the **actual 16px raster** 8x with no resampling, so the decision is
made on the pixels that land rather than on a smooth preview of them.

Two of those sheets changed an opinion, which is the point of rendering them:

- At a 4-unit counter the arm **fuses with the top bar at 16px** — the mark
  stops being an F at the size it is used at most.
- A heavier wordmark is *worse* small, not better: at stroke 11 the `e`'s
  aperture and the `p`'s bowl close up by 96px wide, the stated lockup minimum.

## Clear space and minimum sizes

- **Clear space:** one bar of the mark — one eighth of the mark's height — on
  all four sides. Nothing else sits inside it, including the edge of a
  containing box.
- **Mark minimum:** 16px. At 16 and 32 the browser must be given
  `favicon-16.png` / `favicon-32.png`, which are their own drawings; the 64-unit
  mark scaled to 16 turns the gap and the overhang into grey mush.
- **Lockup minimum:** 96px wide. Below that, drop the wordmark and use the mark.

## Misuse

Each of these breaks something specific, not just taste:

- **Do not** recolour the mark into a two-colour pair that is not `ember` on
  `ink` or `ember-deep` on white. Those two pairs are the ones that have been
  measured against AA.
- **Do not** put the dark variant on a light background or vice versa. The
  accent drops to 3.1:1 the wrong way round.
- **Do not** scale the 64-unit mark below 32px. Use the size-specific drawings.
- **Do not** stretch, shear, rotate, outline, or add a gradient, shadow or
  bevel. The mark is three rectangles; there is nothing to decorate.
- **Do not** close the gap between the stem and the ember row, or re-attach the
  row to the stem. That gap is the idea.
- **Do not** set the wordmark in a font. It is drawn. Typing "fireup" in Inter
  is not the wordmark.
- **Do not** place the mark on a busy photograph. Use the mono variant on a flat
  field instead.

## Palette

Every pair below is computed by `npm run contrast`, which fails the build if one
regresses. "Min" is the AA threshold for how the pair is used: 4.5 for body
text, 3.0 for large text and for edges that have to be perceivable.

| Token | Hex | Used for |
|---|---|---|
| `ink` | `#0B1220` | Page background, dark sections |
| `ink-raised` | `#141E31` | Panels and terminal blocks on ink |
| `ink-muted` | `#4A5568` | Secondary text on light |
| `paper` | `#FFFFFF` | Light background, text on ink |
| `paper-sunk` | `#F5F7FA` | Recessed light sections |
| `paper-muted` | `#AEBBD0` | Secondary text on ink |
| `ember` | `#FF5A2B` | Accent **on ink only** — 3.11:1 on white |
| `ember-deep` | `#C2410C` | Accent on white: links, button fill, the light mark |
| `azure` / `azure-deep` | `#5AA9FF` / `#1D4ED8` | Secondary accent, links |
| `ok` / `ok-deep` | `#4ADE80` / `#15803D` | Status: supported |
| `warn` / `warn-deep` | `#FBBF24` / `#A16207` | Status: partial or unproven |
| `border-dark` / `border-light` | `#6B7B96` / `#767F8C` | Edges that carry meaning (≥3:1) |
| `rule-dark` / `rule-light` | `#22304A` / `#E3E8EF` | Decorative hairlines only |

Highest-traffic pairs: ink/paper **18.72:1**, ink/ember **6.02:1**,
paper/ember-deep **5.18:1**, ink/paper-muted **9.65:1**.

Status colour never carries meaning on its own: every status on the site is
written out in words and carries a glyph as well, so it survives a greyscale
print and the common colour-vision deficiencies.

## Typography

No font files ship, and none are licensed. The stack asks for Inter and falls
back to the platform UI font, so there is no webfont licence to clear and no
third-party request when the page loads. The wordmark is drawn as paths, so the
one piece of type that has to be exactly right does not depend on a font being
installed at all.

The scale is a 1.2 (minor third) ratio on a 16px base; see `size` in
`tokens/brand.json`.

## Open flags for the CEO

1. **Name collision.** `fireup.pro` is an active software-development company
   (Poland, trading since 2013) using the Fireup name in the software services
   category, with black-and-yellow branding. This palette is deliberately not
   black-and-yellow. I am flagging the name, not clearing it — trademark
   clearance is not mine to give and this needs a real search before the name
   goes on anything public.
2. **No licence file.** The repository has no `LICENSE`, so the site does not
   state one. It should before the site is published anywhere.
3. **Sign-off.** The mark, wordmark and palette need approval before they are
   locked. After that, changes come back through the CEO rather than being
   edited here.
