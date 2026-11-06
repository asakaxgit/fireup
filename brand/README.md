# Fireup brand guide

**Status: approved and locked — the mark, the lockup and the palette.**
The lockup was signed off on **2026-11-06** at round 13: *"Signed off — this is
the lockup."* The mark had already been settled at round 5. Nothing in the
identity changes from here without going back through the CEO, and that is
enforced rather than promised: `npm run lock` compares every approved drawing
against `lock/signed-off.json` element by element and fails the build on a
change, naming the shape and the two values. See
[Changing something that is locked](#changing-something-that-is-locked).

The round-5 mark — the F with the fire on the end of its bar, no muzzle — came
back as *"A is perfect"*, so the drawing in `logo/mark.svg` is settled and does
not change again without going back through the CEO. Everything since is the
lockup and the word around that frozen mark: round 6 made the mark the word's
initial letter rather than an object standing beside it, and round 7 answered
*"add accent curve arrow above the u or up. U can be capitalized"* — the word is
now **FireUp**, with an ember accent over the U. Round 8 drew the other reading
of that note, the big arc over the whole *"Up"*, and the answer on it was
*"the gesture is too much"*; round 9 is round 7's accent restored, kept now by
decision rather than by default. Round 10 answered *"tighten the gap of arrow
and U"* and found a measurement error doing it — the accent had been clearing
the letter by **7 units while every comment said 3** — so the same drawing now
sits 4 units lower, at a gap that is measured off the ink rather than off a
construction line. Round 11 took the three answers on round 10's card: the gap
to **2**, the i's dot down to **3** so the word keeps one accent clearance
instead of two, and the capital U's stems up to the cap line so the two
capitals align. Round 12 is the first horizontal note — *"align left start of
arrow and U's start"* — and moves the accent one unit right, so the tail's cap
and the U's left stem share one edge. Round 13 is the first to **redraw** the
accent rather than move it: the chord comes in from 30 to 28.55 so the arrow is
exactly as wide as the U, flush at both ends — which is the drawing that was
signed off, and it is what the marketing site in `site/` uses.

Sign-off settles the drawing. It does not settle the two things that were
flagged alongside it and are still open: the **`fireup.pro` name collision** and
the **flame prior art**, neither of which is mine to clear. Both are legal
questions and both want a real search before this identity goes on anything
published — a deployed site, an announcement, a store listing, an avatar. See
[Open flags for the CEO](#open-flags-for-the-ceo).

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
x-height, smaller than the 16px body text next to it. Round 8's arc moved it
again, to 1.80:1, and took the header to 72px and the phone header to 58px;
round 9 put the box and both heights back where round 7 had them. The rule that
outlived the round trip: the header is sized by **height**, so the word only
holds its optical size if the height follows the envelope. 64px and 72px are
both 0.61px per unit.

## The accent over the U

Rounds 7, 8 and 9, on the note *"add accent curve arrow above the u or up. U can
be capitalized"*. That note had two readings and both were drawn.

Round 7 drew it as an **accent** — a 30-unit chord, stem to stem on the U, sized
to sit in the word the way the tittle does (`scripts/arrow.mjs` + `arrow/`).
Round 7's sheets offered the other reading beside it, the answer took it, and
round 8 drew the **gesture**: an arc spanning the whole of *"Up"*, a 76-unit
chord, the largest object in the identity (`scripts/gesture.mjs` + `gesture/`).
The answer on that was **"the gesture is too much"**, so the shipped drawing is
the accent again.

**That is round 9, and it is not a revert of a mistake.** The gesture was drawn
at full quality, costed, rendered at the sizes it would be used at, and chosen
against — which is what the sheets are for. Both are kept: `arrow/` and
`gesture/` are the record, and `npm run gesture` still renders all four chords
side by side.

The mark is untouched in all four rounds. Every favicon, app icon and mark
variant in `site/assets/` is byte-for-byte what round 6 shipped.

### Round 10: the gap was 7, not 3

The note was **"tighten the gap of arrow and U"**, and it turned out to be a
correction rather than an adjustment.

Rounds 7 to 9 placed the accent 3 units above `ascTop - stroke / 2` — y=0,
where a capital's ink is assumed to begin. The U's stems are **butt-capped
verticals**: they terminate *on* the ascender line instead of straddling it, so
the letter's topmost ink is at y=4 and the arrow was clearing it by 7. The
tittle is the same shape over the same kind of stem, so `ACCENT_CLEARANCE` —
computed from the construction lines in `scripts/word.mjs` — has always been 4
units short of the air it describes. Both halves of the rule were wrong by the
same amount, so the arrow and the tittle really did keep the same clearance as
each other; what was broken was the number, not the consistency.

The error does not spread past accents set over butt-capped stems. The other
place the identity claims a 3-unit clearance — round 6's rule that the mark's
bar must clear the top of the tittle by 3 — measures **3.03 units** in the
shipped lockup, because a horizontal bar's underside is a real ink edge. So the
same "3 units" appeared twice in one drawing at two visibly different sizes,
which is the most likely reason the accent read as loose beside everything else.

`npm run tighten` measures both edges off the rendered alpha every run and
renders the options (`tighten/`). **3 units of measured air ships**: the
tightest value that still holds a slot of background at the 96px lockup
minimum — at 2 the tail and the U's left stem share a pixel there, and at 0 the
accent stops floating and grows a hook off the letter. It is also, for the
first time, the number the drawing has been claiming since round 7.

Round 10 left two things open rather than settling them — the tittle still
keeping 7 where the arrow kept 3, and the capital U's ink stopping 4 units
below the capital F's. Both were answered on its card, and both are round 11.

### Round 11: the three answers, and the one that was not free

The card came back with *"tighter still — take it to 2"*, *"bring the dot down
to 3 too"* and *"raise the U's stems to the cap line"*. `npm run align`
renders them (`align/`) and measures the result.

**Tightening the gap moved the accent up, which is worth stating because it
reads as a contradiction.** Raising the U's stems to the cap line removes
exactly the 4 units round 10 found: the letter grows upward into the slot. Had
the arrow held still it would have ended up 1 unit *inside* the letter. So it
follows the letter up 4 and comes back down 1 for the tighter gap — net 3 units
higher, 1 unit closer. The drawing itself is unchanged: every y in both of its
paths moved by the same whole −3, so this is round 7's approved arrow at a
different height rather than a redrawing of it.

The gap is now **2 units exactly**, by construction rather than approximately —
the tail is a butt cap on a horizontal, so its ink runs to −2, and the U's
stems terminate on 0. Measured independently on the built
`site/assets/fireup-lockup-dark.svg` it comes back at **2.01**; the dot's own
gap at **3.01**.

**The two accents are deliberately not the same number.** The tittle is a
circle over a flat stem, so its slot pinches at one point and opens out on both
sides — 3.0 at its tightest, 3.9 averaged over its run. The arrow's tail is
flat over a flat cap, so its slot is parallel and its mean air *is* its
minimum. Equal minimums would not have read as equal air. Matching the number
instead, the dot at 2, is drawn as `align/accents-dark.png` E.

**The dot was not a free line, and this is the part worth reading.** The lockup
sizes the mark by clearing that dot: `scripts/build.mjs` scaled the F until its
bar underside cleared the tittle by 3. Dropping the dot 4 units would have had
that rule return a mark **8% smaller** — a 2.01:1 lockup gone to 2.14:1, taking
the site header, the clear-space unit and the social card with it. Rendered at
the 96px minimum the smaller mark is also simply worse: the F's counter muddies
and the bar loses its edge (`align/zoom-size.png`).

So the 3 units are now a **floor the mark has to clear, not a size to re-derive
from every time a letter moves**. `SCALE` is frozen at the approved 1.5 and the
equation is asserted instead of solved — it still catches a drawing change that
puts the dot into the fire, and it no longer resizes an approved mark as a side
effect of moving a dot. The bar now clears the dot by 7.

**What it costs downstream: nothing.** The lockup's viewBox is unchanged, so
the header sizing, the clear-space rule and the social card layout are exactly
as they were. The mark, both favicons, every app icon and all three mark
variants are byte-for-byte unchanged. The word-alone variant's own viewBox
followed the accent up 3, which reaches nothing else.

### Exploration sheets are pinned to their own round

Round 11 is the round that proved they had to be. Moving the tittle and the U
re-rendered **108 sheets** across `arrow/`, `gesture/`, `mix/` and `tighten/`
with letters those rounds never drew — round 10's own sheet came back measuring
3 units of air under a tittle it captions as keeping 7. A sheet is the evidence
for a decision that has already been made, so it has to keep showing what was
shown.

`word.mjs` now carries a `HISTORY` of letters as earlier rounds drew them, and
each of those scripts pins itself with `const AS_OF = 'round-10'`. Two things
leaked and had to be pinned as well as the glyphs: `ACCENT_CLEARANCE`, which
rounds 7 and 8 used to *place* the arrow rather than merely to describe it, and
the round-10 rows inside `align.mjs` itself, which read the live dot and so
stopped depicting round 10 the moment the master was edited.

Round 12 is the first test of that: it moved the accent sideways, and the 24
sheets in `align/` came back **byte-identical** without a pin being added.
`align.mjs` never read the glyph's position out of the master — it places the
arrow by the rule the round was drawn to, which is now a horizontal rule as
well as a vertical one. A pinned coordinate would have worked today and gone
stale at the next horizontal note.

Round 13 is where that stops being enough. A rule can absorb a drawing being
*moved*; it cannot absorb the drawing being *redrawn*, and round 13 shortened
the chord. So `align.mjs` and `edge.mjs` now pin the arrow itself — `round-11`
and `round-12` in `HISTORY`, one per round they document — and both directories
still come back byte-identical.

### Round 13: the accent is the width of the letter

Two answers came back on round 12's card — *"back to round 7's centring"* on the
lockup, and *"draw it"* on whether to make both ends flush, which costs a redraw
of the chord. `npm run flush` renders it (`flush/`).

**Those two answers stop disagreeing once the arrow is the right width.** An
object the same width as the letter is centred on it exactly when its edges line
up, so round 13's drawing satisfies round 7's centring rule, round 12's
flush-left rule and flush-right, all at once. There is no horizontal placement
question left to have.

The arrow was **39.4 wide against the U's 38.0**, because the head's corner
overhangs the chord. Only two things can give up that 1.4: the chord, or the
head. Taking it off the head puts it at 2.4 shafts, under the 2.5 this guide
already names as the floor where a head reads as a comma — so it comes off the
chord, which gives up 5% of its length and nothing else. The rise and the head's
length were both scanned and neither reaches flush in either direction, because
the rightmost ink is the head's *corner*: flattening or steepening the curve
only swings that corner further out.

The chord is **28.55**, not 28.5. It is solved rather than subtracted: every
coordinate goes into the master at a tenth of a unit, so the head's corner moves
in steps, and 28.55 is the run that lands it *on* x=154 instead of near it.

#### Two defects this round found

**Round 12's sheets are drawn one unit right of their own captions.**
`edge.mjs` took the arrow's position from a constant, `arrowLeft: 115`, and
built each option as an offset from it; transcribing the answer moved the tail
to 116, and re-running the script re-rendered all thirteen sheets a unit right
of their labels. Measured on the committed `edge/detail-dark.png`: row A's tail
is +1.30 units from the guide against a caption of *flush*, and row C's is
+0.26 against a caption of *1 unit left*. So the row that lines up in the
picture is the one labelled C. **Those sheets are left exactly as they were
rendered** — they are what was actually reviewed, and correcting them would
delete the evidence the answer was given against; the arrow is pinned to round
12's drawing so they keep reproducing byte-for-byte. The note is at the top of
`edge.mjs`.

**Round 13's own first cut was a tenth of a unit off flush.** The placement was
taken from a raster probe, and `contentBox` rounds outward to whole sample
pixels — 0.166 units at that viewBox, sixteen times coarser than the tenth it
was then snapped to. The tail came out at 116.1 against the U's 116, in the
round whose entire subject is being flush. Nothing on any sheet showed it; the
built-asset check below is what caught it.

Both have the same shape, and the rule that comes out of them is: **an option is
built from its rule, and the raster is the check on the geometry rather than its
source.** A sheet composed from "the master, plus an offset" is only correct
until the master moves, which is exactly when it is being looked at. Measuring
is still the right instinct — measuring at a resolution coarser than the claim
is the mistake.

#### The asset is checked, not just the master

`npm run flush` asserts that `logo/wordmark.svg` draws what the script printed,
and then isolates the accent and the U *in the built*
`site/assets/fireup-lockup-dark.svg`, renders them, and reads the edges off the
alpha. Left **+0.00**, right **+0.00**, vertical slot **1.96** — round 11's
answer, unmoved by a horizontal round. It throws on any of those. Every round
since 7 has claimed a number about the shipped lockup in a comment; this is the
first that fails when the claim stops being true, and it is what caught the
116.1.

**What it costs downstream: nothing.** The arrow's ends sit well inside the word
and it got *narrower*, so the lockup's viewBox is unchanged and the header
sizing, the clear-space rule and the social card are exactly as they were. The
mark, both favicons, every app icon and all three mark variants are byte-for-byte
unchanged, as are all 108 historical exploration sheets.

### Round 12: where the accent starts

The note was **"bit fix. align left start of arrow and U's start"**.
`npm run edge` renders it (`edge/`) and reads both edges off the raster.

It is a change of rule rather than a correction. Round 7 placed the accent by
centring its *measured box* on the U's, and because the head overhangs the
chord the arrow is the wider of the two — so centring left it 1 unit outside the
letter on the left and 0.5 on the right. Both of those edges are flat verticals,
and two flat verticals a single unit apart read as a near miss rather than as an
overhang.

*(Round 12 put that extra width at 1.5 units throughout, from the raster. Round
13 computed it as **1.40**: `contentBox` rounds outward to whole sample pixels
and was inflating it. The 1-unit left overhang the note was about is unaffected
— the tail is a butt cap on a whole coordinate either way.)*

Flush, the accent starts where the letter starts and the whole 1.5 hangs off
the right, which is the direction it already points. The other reading of the
note — the tail's cap on the stem's *centre* line, 4 units further right — is
drawn as `edge/detail-dark.png` B; it lines nothing up with anything and leans
the accent to the right.

**The drawing did not change**: every x in both of its paths moved by the same
whole +1. Measured on the built `site/assets/fireup-lockup-dark.svg`, the two
left edges differ by **0.00 units** and the right by 1.50; the vertical slot is
still **1.96**, round 11's answer, untouched. At 320px — the largest real use —
the old offset was 1.5 device pixels; at the 96px minimum it was 0.45, which is
why this is decided at 320.

**What it costs downstream: nothing.** The arrow's own ends sit well inside the
word (the p's bowl is the rightmost ink), so the lockup's viewBox is unchanged
and the header sizing, the clear-space rule and the social card are exactly as
they were. The mark, both favicons, every app icon and all three mark variants
are byte-for-byte unchanged. `edge.mjs` asserts both of those rather than
claiming them: it throws if an option moves the envelope or the vertical slot.

What the accent is, and why each number is what it is:

- **The shaft is 8 — the same as every letter.** The word is a monoline, and
  its one existing accent, the i's tittle, is an 8-unit circle. A second weight
  is a second voice in a one-voice drawing. It does **not** scale with the
  arc's length: in the first cut of round 7 it did, which quietly made a longer
  arc a heavier drawing rather than a bigger one.
- **It clears the U's ink by 2 units** (round 11; round 10 drew 3, and rounds 7
  to 9 drew 7 while calling it 3 — see below). At the 96px minimum that slot is
  one device pixel, so the renderer is deciding and the drawing is not; that
  cost was costed on round 10's sheet and accepted. Taken to 0 the tail fuses
  with the U's left stem and the letter grows a hook.
- **The chord is 28.55 across and 12.28 up** (round 13; it was the U's 30-unit
  stem pitch from round 7 to round 12). It is sized to the letter it sits over,
  which is what makes it an accent rather than a second object.
- **It is exactly as wide as the letter, and starts where the letter starts**
  (round 13): the arrow's ink box and the U's are the same box, x=116 to x=154.
  Rounds 7 to 11 centred a drawing 1.4 units too wide, which left it hanging off
  both ends; round 12 aligned the left edges and sent all 1.4 off the right.
  Round 13 took the width off the chord instead, so all three alignment rules —
  flush left, flush right, centred — now give the same answer.
- **The head is 2.8 shafts wide and 1.7 long**, square to the arc's end tangent.
  Below about 2.5 it reads as a comma.
- **It bows like a growth curve** — flat out of the tail, turning up at the
  head — and points up and to the right, the way the fire blows off the bar.
  Bowed the other way it reads as arriving and flattening out.

**What the accent costs: nothing downstream.** It fits inside the envelope the
fire had already pushed out, so the lockup's viewBox is unchanged from round 6,
the header sizing is unchanged, and the social card is unchanged. The 76-unit
arc is what did not fit: it tipped **12.9 units above the fire** and took the
lockup to 1.80:1, which is the cost the answer declined. Round 8 measured the
rest of it too — the arc kept **55 units of air** from the fire, a distance
`scripts/gesture.mjs` takes off the two rendered layers rather than eyeballing,
and at the 96px minimum it was the most legible of the four chords simply
because it was the largest (`gesture/zoom-chord.png`, four chords as real
pixels). Legibility at the minimum is one input; it did not outrank the brief.

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

- **The 76-unit arc over the whole "Up"** (round 8, `gesture/` row A) — it
  reads, and at the 96px minimum it is the most legible of the four. It was
  declined on what it costs: it becomes the largest object in the identity,
  bigger than the mark, and it takes the lockup's envelope off the fire and
  moves the header sizing with it.
- **A 48-unit chord**, the midpoint, centred over "Up" or over the U
  (`gesture/` rows B and C). Centred over the U it has no anchor: it starts
  above the `e` and ends over the gap, pointing at nothing in particular.
  Centred over "Up" it is neither the accent nor the gesture.
- **Touching the letter, no clearance** — the mark's own rule is *nothing
  floats*, so this was worth drawing. At 96px the tail fuses with the U's left
  stem and the letter grows a hook.
- **A chord inside the stems** — the head ends up as long as the shaft.

**This is a logotype decision, not a spelling decision**, and that was confirmed
rather than assumed: the product stays **"Fireup"** in prose, in the site copy,
the README and the CLI. A logotype styles a name; it does not rename it.

### How the mark got here

The direction is the CEO's, in six steps: round 2's lifted-arm F came back as
*"use fire"*, round 3's burning top bar came back as *"treat F as a lighter gun
and blow fire from right top side"*, round 4's lighter came back as *"4a is
almost good. Omit nozzle entirely"*, round 5 came back as *"A is perfect. Mix
this F into fireup instead of locating next"*, and round 6 came back as
*"almost good. Add accent curve arrow above the u or up. U can be
capitalized"*. Round 7's card then chose the gesture over the accent, round 8
drew it, and the answer on round 8 was *"the gesture is too much"* — so round 9
is round 7's accent, kept on purpose. The mark has been frozen since round 5;
everything after it is the word around it.

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
| `logo/wordmark.svg` | **Master.** "FireUp" and the accent over its U, drawn as paths, not set in a font. |
| `logo/favicon-16.svg` | **Master.** Its own drawing, cut to a 16×16 pixel grid. |
| `logo/favicon-32.svg` | **Master.** Its own drawing, cut to a 32×32 pixel grid. |
| `tokens/brand.json` | **Master.** Colour, type scale, spacing, radii. |
| `tokens/brand.css` | Generated. CSS custom properties, same values. |
| `lock/signed-off.json` | **The approval record.** The signed-off geometry of the mark, the lockup, the two favicons and the palette, one line per drawn element. Written only by `npm run lock -- --adopt`. |
| `explore/` | Round 1–2. The rendered directions that were considered, including the rejected ones. |
| `tune/` | One sheet per tuning axis of the round-2 mark, for choosing between variants by pointing at one. |
| `fire/` | Round 3. The fire concepts, where the fire goes, and the shape of the tongue. |
| `lighter/` | Round 4. The lighter reading: how literal, the jet, the muzzle, and which cues survive. |
| `plain/` | Round 5. With the muzzle gone: where the fire meets the bar, and how far the bar runs. |
| `mix/` | Round 6. Mixing the F into the word: the three directions, how big the F is, and how tight the word tucks. |
| `arrow/` | Round 7. The arrow over the U: what it sits over, the case of the U, how big it is, and the word's own initial. |
| `gesture/` | Round 8. How long the arc is: four chords from the accent to the full "Up", and how much air each leaves the fire. The gesture was chosen here and then declined on cost; round 9 is row D. |
| `tighten/` | Round 10. How much air the accent keeps over the U, in units measured off the ink: six values, and which end of the gap moves. |
| `align/` | Round 11. The three answers applied one at a time; what the dot's new height does to the mark's size; and the two capitals against the F's own cap line. |
| `edge/` | Round 12. Where the accent's left end starts, against the U's own left ink edge. **These thirteen sheets are drawn one unit right of their own captions** — see [Round 12](#round-12-where-the-accent-starts); they are kept exactly as they were reviewed. |
| `flush/` | Round 13. The four ways the accent can become the width of the U, and the scan of which levers reach flush at all. |
| `preview/` | What actually ships, at the sizes it ships at. Rendered from `site/assets/`, not from the masters. |
| `../site/assets/` | Generated. Every variant, lockup, favicon, app icon and the social card. |

Masters are hand-authored and are the only files to edit. Everything else is
output: **do not hand-edit a generated SVG, PNG or CSS file** — the next build
will overwrite it.

## Rebuilding everything

```bash
cd brand
npm install        # @resvg/resvg-js, the raster pipeline
npm run check      # smoke the pipeline, verify contrast, rebuild, check the lock
```

`npm run check` is `smoke` + `contrast` + `build` + `lock`:

- `npm run smoke` renders a known SVG and asserts the PNG really is the
  requested size with the expected pixels. It is there because "the export
  script ran without erroring" is not evidence that it rendered anything. It
  also widens each master's viewBox, re-measures, and fails if the drawing runs
  outside the box it declares — round 4 shipped a flame that did, and the
  renderer silently cut it flat.
- `npm run contrast` computes every palette pair the brand uses and fails if one
  misses its WCAG AA threshold. Nothing is eyeballed.
- `npm run build` regenerates `tokens/brand.css` and all of `site/assets/`.
- `npm run lock` compares the approved drawings against `lock/signed-off.json`
  and fails on any change, naming the element and both values. It runs last,
  after `build`, so it sees the assets the build just wrote rather than
  whatever was in the tree. It locks the two masters, the two favicons, the
  **built** dark lockup — because `build.mjs` decides the 1.5x scale, where the
  word starts and where the accent lands, none of which a master pins — and the
  colour tokens. It does not look at comments, prose or whitespace: the masters
  are meant to be annotated by hand, and a note about why a curve sits where it
  does should never fail a build.
- `npm run explore` re-renders the round 1–2 exploration sheets.
- `npm run tune` re-renders the four tuning sheets in `tune/`.
- `npm run fire` re-renders the round 3 sheets in `fire/`.
- `npm run lighter` re-renders the round 4 sheets in `lighter/`.
- `npm run plain` re-renders the round 5 sheets in `plain/`.
- `npm run mix` re-renders the round 6 lockup sheets in `mix/`.
- `npm run arrow` re-renders the round 7 arrow sheets in `arrow/`, and prints
  the shipping arrow's geometry so the numbers in the master are transcribed
  rather than worked out twice.
- `npm run gesture` re-renders the round 8 chord sheets in `gesture/` — all four
  chords including the shipped accent — and measures the distance between each
  arc and the fire off the two rendered layers rather than by eye.
- `npm run tighten` re-renders the round 10 sheets in `tighten/`, prints the
  air between accent and letter in device pixels at every width the lockup is
  really used at, and prints the shipping arrow's geometry. `SHIP_AIR=<units>`
  prints it for a different gap without changing what the sheets show.
- `npm run align` re-renders the round 11 sheets in `align/`, measures both
  accents in device pixels at every real width, and prints what goes into the
  master. It places the arrow by reading the glyph out of the master and
  measuring it, so its sheets come out **byte-identical whether it is run
  against round 10's master or round 11's** — which is a standing check that
  the transcription into the drawing landed where the sheet said it would.
- `npm run edge` re-renders the round 12 sheets in `edge/`. **Read the defect
  note at the top of the script before using them**: every row is drawn one
  unit right of its own caption.
- `npm run flush` re-renders the round 13 sheets in `flush/`, scans which of
  the arrow's levers can reach flush at all, and — the part worth keeping —
  isolates the accent and the U *inside the built* `site/assets/`
  `fireup-lockup-dark.svg`, renders them, reads the edges off the alpha and
  throws if they are not flush. Every round since 7 made a claim in a comment
  about the shipped lockup; this is the first one that fails when the claim
  stops being true.
- `npm run preview` re-renders `preview/` from the built assets.

`explore`, `tune`, `fire`, `lighter`, `plain`, `mix`, `arrow`, `gesture`,
`tighten`, `align`, `edge` and `flush` are
deliberately outside `check`: they are the record of a decision, not shipped
assets, and they should only change when the question being asked changes. Each
one is pinned to its own round — `fire` and `tune` hold their own copy of the
drawings they compare, and the rest ask `word.mjs` for the letters and the
arrow **as that round drew them**, so editing the identity does not rewrite the
evidence an earlier answer was given against. That was learned the hard way
twice: see
[Exploration sheets are pinned to their own round](#exploration-sheets-are-pinned-to-their-own-round).
`preview` is outside `check` because looking at the result is a human step, not
a gate.

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
4. ~~**Sign-off.**~~ **Closed 2026-11-06** — *"Signed off — this is the
   lockup."* The mark, the lockup and the palette are approved and frozen in
   `lock/signed-off.json`. Changes now come back through the CEO rather than
   being edited here, and `npm run lock` is what makes that true rather than
   stated; see
   [Changing something that is locked](#changing-something-that-is-locked).

Flags 1 to 3 are not mine to close. 1 and 2 are legal opinions; 3 is a
repository-level decision. All three gate **publishing** the identity — a
deployed site, an announcement, a store listing, an avatar — rather than
keeping it in this repository, where it already is with the CEO's approval.

## Changing something that is locked

The identity is approved, so a change to it is a decision, not an edit. The
order is: ask the CEO, get an answer, make the change, then re-freeze with the
answer attached —

```bash
npm run lock -- --adopt "round 14 signed off: <the approval, in its own words>"
```

`--adopt` refuses to run without that text, so the lock file's `history` is a
record of what was approved and when rather than a list of times the check was
silenced. It rewrites the geometry from the current drawings in one go, which
means the diff a reviewer sees is exactly the coordinates that moved.

What the lock does **not** cover, by design: comments, prose, whitespace, the
exploration sheets, the site, and anything outside the attribute whitelist in
`scripts/lock.mjs`. It is the drawing that is frozen, not the file.

It has been proved to fail, in both of the ways it needs to:

- moving the F's middle arm one unit in `logo/mark.svg` — caught, with the
  element and both values printed;
- changing `SCALE` in `build.mjs` from 1.5 to 1.52 and rebuilding, which leaves
  **every master byte-identical** — caught on the built lockup, which is the
  case the masters alone cannot see and the reason that asset is locked too.
