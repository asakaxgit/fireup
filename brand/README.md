# Fireup brand guide

**Status: proposed, redrawn to the CEO's note, still not approved.** The mark in
this directory is round 3 for SAK-173: the CEO reviewed round 2 and asked for
fire, and for the wordmark to stay at the lighter weight. Both are done. It is
in use on the marketing site in `site/`, but **approval has not been given** —
identity direction is the CEO's sign-off and it is still open.

This directory is in the public repository so the direction can be reviewed in a
pull request. That is not approval. Until the mark is signed off, do not use it
as the company's mark anywhere outside this repo — no published site, no
external announcement, no profile or avatar.

---

## The mark

An **F** whose top bar is alight. The bar runs out from the stem and its far end
rises into a tongue: the fire is a terminal of the letterform, not an object
balanced on top of it. Fireup is what you run to bring a local stack up, so the
mark is the moment it catches.

It is a monogram first. That is what lets it still say the name at 16px, where a
picture cannot.

### How it got here

The direction is the CEO's: an earlier mark — an F with its middle arm lifted
clear of the stem — was reviewed and the note back was *"use fire"*. Three fire
concepts were drawn and rendered before this one was cut, and the rendering
changed the answer twice. All of it is in `scripts/fire.mjs` and `fire/`:

- **Fire built from stacked rows** reads as a stepped pyramid, not as fire, and
  at 16px the 6-unit bars land on 1.5 pixels and smear to grey.
- **A free-standing flame on a plinth** renders as a **water droplet**, at every
  size, on both backgrounds.
- **A flame on the F's middle arm** curls back under the top bar and encloses a
  bowl, so the mark reads as a **P**. Three tip heights, all three a P. That is
  why the fire ended up on the top bar: an F's counter has to stay open on the
  right.
- **The tongue rises before it leans.** Flaring it sideways first makes the mark
  right-heavy and turns the flame into a swoosh.

### The trademark position, stated plainly

A flame is the single most confusable shape available to a Firebase-compatible
tool, and this was flagged before the direction was chosen:

- Firebase's own mark **is a flame** — three faceted segments on a
  red-orange-yellow gradient — and Google's brand guidelines forbid marks that
  could be confused with it.
- Tinder enforces a trademark over a rounded flame outline and has sent
  infringement notices over flame icons in software.

The CEO directed fire with that on the table, so the drawing answers the risk
instead of the risk vetoing the drawing. This mark is **not** a flame
silhouette: there is no faceted construction, no gradient, no free-floating
flame. The fire is a stroke terminal of a letter F, in a single flat colour, and
the palette is deliberately not Firebase's amber.

That reduces the risk. **It does not clear it.** Clearance is a legal opinion
and is not mine to give — see the open flags at the bottom of this file.

### What it deliberately is not

- **Not a power symbol.** An early direction — a rounded square with a bar
  breaking out of the top — rendered as the IEC 5009 power glyph. It read as a
  button rather than a brand, and a universal standard symbol cannot be owned.
  It is kept in `scripts/explore.mjs` with that note.
- **Not Firebase's flame, and not Tinder's.** See above.

## Files

| File | What it is |
|---|---|
| `logo/mark.svg` | **Master.** The mark, 64-unit grid. |
| `logo/wordmark.svg` | **Master.** "fireup", drawn as paths, not set in a font. |
| `logo/favicon-16.svg` | **Master.** Its own drawing, cut to a 16×16 pixel grid. |
| `logo/favicon-32.svg` | **Master.** Its own drawing, cut to a 32×32 pixel grid. |
| `tokens/brand.json` | **Master.** Colour, type scale, spacing, radii. |
| `tokens/brand.css` | Generated. CSS custom properties, same values. |
| `explore/` | Round 1–2. The rendered directions that were considered, including the rejected ones. |
| `tune/` | One sheet per tuning axis of the round-2 mark, for choosing between variants by pointing at one. |
| `fire/` | Round 3. The fire concepts, where the fire goes, and the shape of the tongue. |
| `preview/` | What actually ships, at the sizes it ships at. Rendered from `site/assets/`, not from the masters. |
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
- `npm run explore` re-renders the round 1–2 exploration sheets.
- `npm run tune` re-renders the four tuning sheets in `tune/`.
- `npm run fire` re-renders the round 3 sheets in `fire/`.
- `npm run preview` re-renders `preview/` from the built assets.

`explore`, `tune` and `fire` are deliberately outside `check`: they are the
record of a decision, not shipped assets, and they should only change when the
question being asked changes. `preview` is outside it because looking at the
result is a human step, not a gate.

## Deciding by looking, not by arguing

Every sheet puts three options side by side at the sizes they will really be
used at, and for the mark it also magnifies the **actual 16px raster** with no
resampling — rendered at 16, read back out of the renderer, redrawn as one
square per pixel. A magnified preview that disagrees with the real output is
worse than no preview.

That has now reversed a stated opinion five times, which is the whole point:

- At a 4-unit counter the arm **fused with the top bar at 16px** — the mark
  stopped being an F at the size it is used at most.
- A heavier wordmark is *worse* small, not better: at stroke 11 the `e`'s
  aperture and the `p`'s bowl close up by 96px wide, the stated lockup minimum.
  Stroke 8 is the most legible there, and is what ships.
- Fire built from rows is a **pyramid**; a flame on a plinth is a **droplet**; a
  flame on the F's middle arm is a **P**. All three were written up as good
  ideas before they were rendered.

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
- **Do not** add a gradient to the flame, or facet it into segments. Both are
  how Firebase's mark is built, and the single flat colour is what keeps this
  one distinguishable from it. This is the one misuse rule with a legal edge.
- **Do not** stretch, shear, rotate, outline, or add a shadow or bevel. The mark
  is two rectangles and one path; there is nothing to decorate.
- **Do not** detach the flame from the top bar, or use it on its own as an icon.
  Separated, it is a generic flame silhouette and loses both the idea and the
  distance from the marks named above.
- **Do not** let the middle arm grow to the width of the top bar. The counter
  closing on the right is what turned earlier drawings into a P.
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
2. **Flame prior art.** The mark is now fire, by direction. Firebase's own mark
   is a flame and we are a Firebase-compatible tool; Tinder separately enforces
   a flame-icon trademark. The drawing is built to sit as far from both as a
   flame can — letterform terminal, flat colour, no facets, no amber — and that
   is risk reduction, not clearance. A real search is warranted before the mark
   goes on anything published, and I am not qualified to give the opinion.
3. **No licence file.** The repository has no `LICENSE`, so the site does not
   state one. It should before the site is published anywhere.
4. **Sign-off.** The mark, wordmark and palette need approval before they are
   locked. After that, changes come back through the CEO rather than being
   edited here.
