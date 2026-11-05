# Fireup brand guide

**Status: the mark is approved; the lockup around it is round 7 and is not.**
The round-5 mark — the F with the fire on the end of its bar, no muzzle — came
back as *"A is perfect"*, so the drawing in `logo/mark.svg` is settled and does
not change again without going back through the CEO. Everything since is the
lockup and the word around that frozen mark: round 6 made the mark the word's
initial letter rather than an object standing beside it, and round 7 answers
*"add accent curve arrow above the u or up. U can be capitalized"* — the word is
now **FireUp**, with an ember curved arrow over the U. The lockup is in use on
the marketing site in `site/`, but **sign-off on it has not been given** and is
still open.

This directory is in the public repository so the direction can be reviewed in a
pull request. That is not approval. Until the mark is signed off, do not use it
as the company's mark anywhere outside this repo — no published site, no
external announcement, no profile or avatar.

---

## The mark

An **F whose top bar runs out and catches**, the fire blowing up and to the
right off its end. There is no muzzle and no other object: one bar weight end to
end, and the fire takes over where the bar stops. Fireup is what you run to
bring a local stack up, so the mark is the moment it fires.

It is a monogram first. That is what lets it still say the name at 16px, where a
picture cannot — the counter stays open on the right, so nothing about the fire
stops it being an F.

## The lockup

The lockup is not the mark plus the word. It is the word **FireUp**, with the
mark as its **F**: the mark stands on the wordmark's baseline at 1.5x, its bar
and fire overhang the lowercase, and the wordmark's own monoline `F` is not
drawn, because that letter is now the mark. One object, read once.

The three numbers that make it work, all in `scripts/build.mjs`:

- **How big the F is — cap 1.43x the word's ascender.** Not a taste decision.
  The bar and fire occupy the whole band above x-height, and the one thing
  reaching up into that band is the **i's tittle**. The F is scaled until the
  bar's underside clears the top of the tittle by 3 units; below a 0 clearance
  the dot lands inside the fire and the two merge into one blob by 128px.
- **Where the word starts — 8 units after the F's middle arm.** That is the
  whitespace the wordmark already keeps between its own letters (the i's stem
  ends at 38, the r's begins at 46), so the F is spaced like a letter and not
  like a mark. The word cannot start earlier: below the arm, the arm and the
  lowercase x-height band want the same nine units of height.
- **The box is measured, not counted.** The viewBox comes from the rendered
  alpha, so the flame's overshoot above the cap and the `p`'s descender below
  the baseline both set it.

Round 6 is `scripts/mix.mjs` and `mix/`. Two directions were drawn and
rejected against this one:

- **Inline**, the F at exactly the word's ascender height. Reads as one
  logotype at 320px, but the fire and the tittle end up side by side with
  nothing between them and merge by 128px.
- **Ligature**, the tittle dropped and the i's stem run up into the bar's
  underside. An undotted stem that tall is an **l**: the word reads *Flreup*,
  at 320px as clearly as at 96.

The lockup's aspect went from 3.09:1 to 2.01:1, which is why the site header
grew from a 52px to a 64px logo — at 52 the lowercase landed at a 14.9px
x-height, smaller than the 16px body text next to it.

## The arrow over the U

Round 7, on the note *"add accent curve arrow above the u or up. U can be
capitalized"*. It is `scripts/arrow.mjs` and `arrow/`. The mark is untouched:
every favicon, app icon and mark variant in `site/assets/` is byte-for-byte what
round 6 shipped, and so is the lockup's viewBox — the arrow fits inside the
envelope the fire already set, so nothing downstream had to move.

What the arrow is, and why each number is what it is:

- **The shaft is 8 — the same as every letter.** The word is a monoline, and
  its one existing accent, the i's tittle, is an 8-unit circle. An accent at any
  other weight is a second voice in a one-voice drawing. The weight does not
  scale with the arrow's length.
- **It clears the U by 3 units.** That is the gap the tittle keeps over the i's
  stem — the word's own accent clearance, read off the master rather than
  chosen. Taking it to 0 so the arrow grows out of the letter (7a-C) makes the
  U look like it has sprouted a hook at 96px.
- **The chord runs stem to stem across the U, 30 across and 12.9 up.** Narrower
  and the head is as long as the shaft, with no curve left to read. Out to the
  p and it stops being an accent: a second ember object the size of the fire, at
  the other end of the lockup.
- **The head is 2.8 shafts wide and 1.7 long**, square to the arc's end tangent.
  Below about 2.5 it reads as a comma.
- **It bows like a growth curve** — flat out of the tail, turning up at the
  head — and points up and to the right, the way the fire blows off the bar.
  Bowed the other way it reads as arriving and flattening out.

**The U is capitalised and so is the F.** The note allowed the capital U; the
capital F is its consequence rather than a second idea. The lockup's F is the
mark, which is a capital letterform, so the lockup already said *FireUp*. Had
the word's own drawing kept its lowercase `f`, the wordmark variant in
`site/assets/` would have said *fireUp* — two spellings of one name in one
identity. The F needed 24 units against its 46-unit cap, which the lowercase
f's slot did not have, so it takes them out of the left margin and every other
letter stays exactly where it was (`arrow/initial-*.png`). Squeezed into the old
slot instead, it comes out 15 units wide and still reads as an `f`.

Rejected on the sheets, with the reason each one died:

- **Over "Up" rather than over the U** — the arc runs the length of both
  letters. It reads, but it rises past the fire's tip and takes the lockup from
  2.01:1 to 1.80:1, and the accent becomes the largest object in the drawing.
  If the gesture is what you want rather than the accent, this is the one to
  ask for; it costs the header sizing again.
- **Touching the letter, no clearance** — the mark's own rule is *nothing
  floats*, so this was worth drawing. At 96px the tail fuses with the U's left
  stem and the letter grows a hook.
- **A chord inside the stems** — the head ends up as long as the shaft.

**This is a logotype decision, not a spelling decision.** The site copy and the
CLI still say "Fireup" in prose, which is normal — a logotype styles the name,
it does not rename it. If the product name itself should become "FireUp"
everywhere, that is the CEO's call and a separate change.

### How the mark got here

The direction is the CEO's, in six steps: round 2's lifted-arm F came back as
*"use fire"*, round 3's burning top bar came back as *"treat F as a lighter gun
and blow fire from right top side"*, round 4's lighter came back as *"4a is
almost good. Omit nozzle entirely"*, round 5 came back as *"A is perfect. Mix
this F into fireup instead of locating next"*, and round 6 came back as
*"almost good. Add accent curve arrow above the u or up. U can be
capitalized"*. The mark has been frozen since round 5; everything after it is
the word around it.

Round 5 is two sheets — `scripts/plain.mjs` and `plain/`. Taking the muzzle out
reopened the one question it had been answering, because the fire used to sit on
a 6-unit nozzle and its 9-unit base was *wider* than what it sat on:

- **5a, the join.** Against the bar's full 12 weight that same base is narrower
  than what it sits on, and the join becomes a visible step — notched 2 units
  above and 1 below. A base spanning the full bar weight has no step at all: the
  bar's end simply is the fire. Lifting the fire onto the top half only, which
  is the most literal reading of *"from right top side"*, leaves the bar's
  bottom-right corner sticking out square beneath it.
- **5b, the bar length.** At 32 the top bar ends only 4 units past the middle
  arm and the fire becomes the widest thing in the mark; at 40 the letter
  outweighs the fire. **36** is what ships.

Round 4 is the lighter reading, kept in `scripts/lighter.mjs` and `lighter/`,
and it is why no part of the appliance survives into round 5:

- **4a, how literal.** A flared foot on the stem reads as a **lamp base**, not a
  grip. The most literal drawing — long thin nozzle, trigger wedge, flared
  grip — is the only one that reads as the object, and what it reads as is a
  **gun**, which is the one read to avoid.
- **4b, the jet.** Fire blown flat along the barrel is a **swoosh**; at 45° it
  is a **leaf**. Only a jet with a belly and a second, shorter tongue reads as
  fire, and the notch between the two is what does it.
- **4c, the muzzle.** A muzzle two units narrower than the 12-weight bar is
  invisible at 512px, let alone small. Three units of daylight between muzzle
  and flame makes the fire start to float free, which is the silhouette being
  avoided.
- **4d, the cues.** A muzzle at **half** the bar's weight was the one cue coarse
  enough to read, and it is the one the CEO's note then removed. The middle arm
  cut as a trigger wedge does *not* read as a trigger — it reads as a
  **fast-forward triangle**, and at 16px it fills the counter.

Round 3's findings still hold and are why the fire is where it is:
fire built from stacked rows is a **pyramid**, a free-standing flame on a plinth
is a **water droplet**, and a flame on the F's middle arm closes a bowl and
makes the mark a **P**. Those are in `scripts/fire.mjs` and `fire/`.

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
| `logo/wordmark.svg` | **Master.** "FireUp" and the arrow over its U, drawn as paths, not set in a font. |
| `logo/favicon-16.svg` | **Master.** Its own drawing, cut to a 16×16 pixel grid. |
| `logo/favicon-32.svg` | **Master.** Its own drawing, cut to a 32×32 pixel grid. |
| `tokens/brand.json` | **Master.** Colour, type scale, spacing, radii. |
| `tokens/brand.css` | Generated. CSS custom properties, same values. |
| `explore/` | Round 1–2. The rendered directions that were considered, including the rejected ones. |
| `tune/` | One sheet per tuning axis of the round-2 mark, for choosing between variants by pointing at one. |
| `fire/` | Round 3. The fire concepts, where the fire goes, and the shape of the tongue. |
| `lighter/` | Round 4. The lighter reading: how literal, the jet, the muzzle, and which cues survive. |
| `plain/` | Round 5. With the muzzle gone: where the fire meets the bar, and how far the bar runs. |
| `mix/` | Round 6. Mixing the F into the word: the three directions, how big the F is, and how tight the word tucks. |
| `arrow/` | Round 7. The arrow over the U: what it sits over, the case of the U, how big it is, and the word's own initial. |
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
  script ran without erroring" is not evidence that it rendered anything. It
  also widens each master's viewBox, re-measures, and fails if the drawing runs
  outside the box it declares — round 4 shipped a flame that did, and the
  renderer silently cut it flat.
- `npm run contrast` computes every palette pair the brand uses and fails if one
  misses its WCAG AA threshold. Nothing is eyeballed.
- `npm run build` regenerates `tokens/brand.css` and all of `site/assets/`.
- `npm run explore` re-renders the round 1–2 exploration sheets.
- `npm run tune` re-renders the four tuning sheets in `tune/`.
- `npm run fire` re-renders the round 3 sheets in `fire/`.
- `npm run lighter` re-renders the round 4 sheets in `lighter/`.
- `npm run plain` re-renders the round 5 sheets in `plain/`.
- `npm run mix` re-renders the round 6 lockup sheets in `mix/`.
- `npm run arrow` re-renders the round 7 arrow sheets in `arrow/`, and prints
  the shipping arrow's geometry so the numbers in the master are transcribed
  rather than worked out twice.
- `npm run preview` re-renders `preview/` from the built assets.

`explore`, `tune`, `fire`, `lighter`, `plain`, `mix` and `arrow` are
deliberately outside `check`: they are the record of a decision, not shipped
assets, and they should only change when the question being asked changes.
`fire` and `tune` hold their own copy of the drawings they compare; `mix` and
`arrow` read the masters, so they re-render when a letterform changes — on
purpose, because a sheet frozen against a drawing we no longer ship is worse
than one that moves. `preview` is outside it because looking at the
result is a human step, not a gate.

## Deciding by looking, not by arguing

Every sheet puts three options side by side at the sizes they will really be
used at, and for the mark it also magnifies the **actual 16px raster** with no
resampling — rendered at 16, read back out of the renderer, redrawn as one
square per pixel. A magnified preview that disagrees with the real output is
worse than no preview.

That has now reversed a stated opinion eleven times, which is the whole point:

- At a 4-unit counter the arm **fused with the top bar at 16px** — the mark
  stopped being an F at the size it is used at most.
- A heavier wordmark is *worse* small, not better: at stroke 11 the `e`'s
  aperture and the `p`'s bowl close up by 96px wide, the stated lockup minimum.
  Stroke 8 is the most legible there, and is what ships.
- Fire built from rows is a **pyramid**; a flame on a plinth is a **droplet**; a
  flame on the F's middle arm is a **P**. All three were written up as good
  ideas before they were rendered.
- A trigger wedge on the middle arm reads as a **fast-forward triangle**, and a
  flared foot on the stem reads as a **lamp base**. Both were drawn to make the
  lighter legible and both made it less so.
- The first round-4 flame ran past the 64-unit viewBox and the renderer **cut
  its outer lobe flat**. Nothing below 512px showed it, which is why the app
  icon is now on the preview sheet at 1:1.
- Running the i's stem up into the F's bar to make a ligature produces an **l**,
  not a dotted i: the word reads *Flreup*. The note calling it "the tightest
  mix" was written before it was rendered.
- The wordmark's letter gap is **8** units, not the 14 of its stem-to-stem
  pitch. Round 6c was built on the wrong number until the sheet was looked at.

## Clear space and minimum sizes

- **Clear space:** one bar of the mark — one eighth of the mark's height — on
  all four sides. Nothing else sits inside it, including the edge of a
  containing box.
- **Mark minimum:** 16px. At 16 and 32 the browser must be given
  `favicon-16.png` / `favicon-32.png`, which are their own drawings; the 64-unit
  mark scaled to 16 turns the gap and the overhang into grey mush.
- **Lockup minimum:** 96px wide. Below that, use the mark alone. At 96 the
  lockup's lowercase is a 13.6px x-height on a 3.6px stroke — the round-6
  lockup is squarer than the one it replaces, so it is *more* legible at that
  width, not less. The site header is 129px wide and the phone header 105px.

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
  is three rectangles and one path; there is nothing to decorate.
- **Do not** detach the fire from the end of the bar, or use it on its own as an
  icon. Separated, it is a generic flame silhouette and loses both the idea and
  the distance from the marks named above.
- **Do not** inset the fire's base from the bar's edges. It spans the bar's full
  12 weight so the join is a seam; at 9 units — which is what sat on round 4's
  muzzle — the silhouette notches 2 units above the bar and 1 below.
- **Do not** move where the bar stops. At 32 the fire becomes the widest thing
  in the mark; past 40 the second tongue runs out of the 64-unit viewBox and the
  renderer clips it flat.
- **Do not** let the middle arm grow to the width of the top bar. The counter
  closing on the right is what turned earlier drawings into a P.
- **Do not** set the wordmark in a font. It is drawn. Typing "FireUp" in Inter
  is not the wordmark, and neither is typing it with a Unicode arrow after it.
- **Do not** write the mark and the word next to each other as two objects, and
  do not draw an `F` in front of the word. The mark *is* the F; a lockup with
  both says the name twice.
- **Do not** redraw the arrow at a different weight, or let it scale with its
  own length. It is 8 units because every letter is, and because the tittle is.
- **Do not** lift the arrow off the U and use it on its own as an icon. It is an
  accent on a letter; the mark is the F.
- **Do not** shrink the F in the lockup to the word's own height, or close the
  gap after its middle arm. Either puts the i's tittle back under the fire,
  where the two merge into one blob by 128px.
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
