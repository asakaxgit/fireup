#!/usr/bin/env node
// Round eleven. Three answers came back on round 10's card:
//
//   "Tighter still — take it to 2"
//   "Bring the dot down to 3 too"
//   "Raise the U's stems to the cap line"
//
//   node scripts/align.mjs   ->  brand/align/*.png
//
// They are three answers to one shape, not three independent edits, and two of
// them pull the same slot in opposite directions. That is what this script is
// for: to apply them in an order that can be looked at, and to measure the
// result rather than assert it.
//
// WHY "TIGHTER" MOVES THE ARROW UP. Round 10's whole finding was that the U's
// stems are butt-capped, so they terminate *on* the ascender line and the
// letter's topmost ink is at y=4 rather than the y=0 the construction assumes.
// Raising the stems to the cap line removes exactly that 4: the letter grows
// upward into the slot. If the arrow held still, the 3 units of air it has now
// would become -1 — the accent would sit inside the letter. So the arrow has to
// follow the letter up 4 and then come down 1 for the tighter gap, and the net
// is 3 units higher than the PR. The gap narrows by one unit while the drawing
// it belongs to rises by three. Neither number is visible on its own; only the
// slot is, and that is measured below.
//
// The other two answers are one line each:
//   * the i's tittle comes down 4, so the word's two accents keep the same 3
//     units of measured air instead of 3 and 7;
//   * the U's stems run to y=0, which is where the capital F's cap bar already
//     puts its ink, so the two capitals in the word-alone variant align.
//
// AND THE TITTLE IS NOT A FREE LINE, WHICH IS 11b. The lockup derives how big
// the mark is from the tittle: scale the F until its bar underside clears the
// dot by 3 (scripts/build.mjs). The dot is the one thing under the bar that
// reaches up into that band, so that rule is what keeps the two off each other.
// Drop the dot 4 and the rule returns a mark 8% smaller, which moves the lockup
// aspect, the site header, the clear-space unit and the social card — none of
// which was asked for. 11b draws both and the card asks. The reading this
// script ships: the rule is a floor the mark has to clear, not a size to
// re-derive from every time a letter moves, so the mark stays the size it was
// signed off at and build.mjs asserts the floor instead of solving for it.
//
// The mark itself is frozen — signed off in round 5, moved into the word in
// round 6, untouched since. Nothing here opens mark.svg.
//
// The arrow is not redrawn either. This script reads the approved glyph out of
// the master and places it by its measured ink, so every option below is the
// same drawing at a different height. After the master is edited to what ships,
// re-running this script has to produce the same sheets — it re-measures the
// glyph it is given — which is a free check that the transcription landed.
//
// Round 12 then moved the accent sideways, and the same mechanism absorbs that:
// the glyph is placed horizontally by round 7's centring rule as well as
// vertically by its air, so these sheets go on showing round 11's drawing
// without pinning a copy of it. See ARROW_DX below.
//
// Wordmark construction (logo/wordmark.svg, via word.mjs): baseline y=50,
// x-height top y=20, ascender top y=4, descender y=66, stroke 8.
// Mark construction (logo/mark.svg): F foot y=60, cap top y=16, bar weight 12,
// middle arm ends x=28, bar ends x=36, fire tip y=1.
import { renderPng, contentBox, pixelMap } from './render.mjs'
import { inkGap, slotRun } from './measure.mjs'
import { master, innerOf, recolour, spell, GLYPHS, W, DOT } from './word.mjs'
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')
const tokens = JSON.parse(readFileSync(join(root, 'tokens', 'brand.json'), 'utf8'))

const INK = tokens.color.ink
const PAPER = tokens.color.paper
const EMBER = tokens.color.ember
const EMBER_DEEP = tokens.color['ember-deep']
const MUTED = tokens.color['paper-muted']

const PAINT = `fill="none" stroke="#FFFFFF" stroke-width="${W.stroke}" stroke-linecap="butt" stroke-linejoin="round"`

// Painted on its own, big, for measuring one glyph's ink.
const alone = (markup) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="-60 -90 340 200"><g ${PAINT}>${markup}</g></svg>`
const inkOf = (markup) => contentBox(alone(markup), 2048)

// --- the two letters that move ------------------------------------------------
// Drawn here rather than taken from the master because these sheets draw them at
// two positions each and the master carries only the one that ships.
//
// `top` is where the U's stems terminate. The shipped letter today runs them to
// the ascender line, 4; the answer is to run them to 0, where the F's cap bar
// puts its ink.
const capU = (top) => `<path d="M120 ${top} V35 A15 15 0 0 0 150 35 V${top}"/>`
const dotAt = (cy) => `<circle cx="${DOT.cx}" cy="${cy}" r="${DOT.r}" fill="#FFFFFF" stroke="none"/>`

const U_TOP = { drawn: inkOf(capU(W.ascTop)).y, raised: inkOf(capU(0)).y }
const F_TOP = inkOf(GLYPHS.get('F')).y
const STEM_TOP = inkOf(GLYPHS.get('i')).y

// MEASURED vs CONSTRUCTED, and why both are printed below. `contentBox` rounds
// outward to whole sample pixels at both edges, so a measured air figure runs
// about 0.15 units under the geometry at this sample size — the U's cap reads
// 0.02 high and the arrow's reads 0.14 low. That is measurement bias, not drift
// in the drawing, and it is the reason this round quotes the construction for
// what it ships and re-measures the built asset separately.
//
// The construction is exact because every edge in question is a butt cap on a
// horizontal: a stem terminating on y=n has its topmost ink at n, and a
// horizontal stroke centred on y=n has its ink from n - 4 to n + 4.
const GEO = { uDrawn: W.ascTop, uRaised: 0, stemTop: W.xTop }
const dotAir = (cy) => GEO.stemTop - (cy + DOT.r)
const measuredDotAir = (cy) => STEM_TOP - (cy + DOT.r)

// The dot's height, solved for rather than typed, then rounded to whole units
// because the master is meant to stay a drawing someone can read and correct.
//
// Named by the air each one holds rather than by a coordinate, so every row
// below means the same thing whichever version of the master this runs against.
// An earlier cut gave the round-10 row `DOT.cy`, "whatever the master says" —
// which stopped being round 10 the moment the master was edited to round 11,
// and silently re-rendered the before-state as the after-state.
const dotFor = (air) => Math.round(DOT.cy + (dotAir(DOT.cy) - air))
const DOT_ROUND10 = dotFor(7) // the before-state: the dot where it kept 7
const DOT_SHIPPED = dotFor(3) // the answer, literally: "bring the dot down to 3"
const DOT_MATCH = dotFor(2) // the other reading: match the arrow's new number

// --- the arrow, which is not redrawn ------------------------------------------
// The approved glyph, lifted out of the master and measured. Only its height
// changes in this round; its shape, weight, chord and head are round 7's and are
// not in question.
const ARROW = GLYPHS.get('arrow')
const ARROW_BOX = inkOf(ARROW)

// Where the glyph's lowest ink sits in the master, as a whole unit. Read from
// the drawing rather than written down, so this script places the arrow at the
// same absolute height whatever the master currently says — which is what makes
// its sheets survive the master being edited to one of them. Written down as a
// constant, every option would have silently moved with the drawing and the
// sheets would have re-rendered three units out.
//
// It is rounded because the master's coordinates are whole units and the
// measurement is not: `contentBox` rounds outward to whole sample pixels, so
// the tail's butt cap reads about 0.14 low.
GEO.arrowBottom = Math.round(ARROW_BOX.y + ARROW_BOX.h)

// And where it sits horizontally — which round 11 did not have to think about,
// because no round before 12 had moved the accent sideways.
//
// Round 12 did: the note was "align left start of arrow and U's start", so the
// master now starts the tail on the U's left ink edge instead of centring the
// arrow's box on the letter's. These sheets are the evidence for round 11's
// three answers, so they have to keep showing round 11's drawing — and the way
// to do that is the way the height already does it, by placing the glyph under
// the rule the round was drawn to rather than by reading a coordinate out of a
// master that has since moved on. That rule was round 7's: the arrow's measured
// box centred on the U's, snapped to the whole unit the master carries.
//
// A hardcoded x would have worked today and gone stale at the next horizontal
// note. Computed, this is dx=0 against round 11's master and dx=-1 against
// round 12's, and the 24 sheets below come out byte-identical either way.
const U_INK = inkOf(capU(0))
const CENTRED_LEFT = Math.round(U_INK.x + U_INK.w / 2 - ARROW_BOX.w / 2)
const ARROW_DX = CENTRED_LEFT - Math.round(ARROW_BOX.x)

// The same glyph with `air` units of background between its lowest ink and the
// top of whichever U it sits over, and at the x round 11 drew it at.
//
// The offset is a whole number of units, so every option is the approved
// drawing moved by an integer and the master keeps readable coordinates. The
// row that asks for today's gap over today's U therefore comes out at dy=0,
// i.e. bit-identical to the master — a free check that this script is placing
// the glyph the same way the drawing does.
const arrowAt = (air, uTop) => {
  const dy = uTop - air - GEO.arrowBottom
  const below = uTop === GEO.uDrawn ? U_TOP.drawn : U_TOP.raised
  return {
    air,
    below,
    dy,
    measuredAir: below - (ARROW_BOX.y + ARROW_BOX.h + dy),
    bottom: ARROW_BOX.y + ARROW_BOX.h + dy,
    top: ARROW_BOX.y + dy,
    draw: (accent, shift = 0) =>
      `<g ${recolour(PAINT, '#FFFFFF', accent)} transform="translate(${(shift + ARROW_DX).toFixed(2)} ${dy.toFixed(2)})">${recolour(ARROW, '#FFFFFF', accent)}</g>`,
  }
}

// --- the lockup, with the mark's size as a variable ---------------------------
const M = { foot: 60, capTop: 16, bar: 12, armEnd: 28, barEnd: 36, fireTip: 1, stemL: 6 }
const FIRE_FOOT = 28 // the underside of the bar, and of the fire sitting on it
const FROZEN_SCALE = 1.5 // round 6, signed off. See 11b.

// What build.mjs's rule returns for a given dot: the smallest mark whose bar
// underside clears the dot's top by `clearance`.
const derivedScale = (dotCy, clearance = 3) =>
  (W.baseline - (dotCy - DOT.r) + clearance) / (M.foot - FIRE_FOOT)

// ...and the air that rule actually buys at a given size, which is the number
// 11b is about. The bar's underside sits at `baseline - (foot - fireFoot) *
// scale`; the dot's top at `cy - r`; the air is the dot's top less the bar's,
// because the bar is the higher of the two.
const barAir = (dotCy, markScale) => dotCy - DOT.r - (W.baseline - (M.foot - FIRE_FOOT) * markScale)

const placed = (markScale) => {
  const markX = (u) => (u - M.stemL) * markScale
  return { markScale, shift: markX(M.armEnd) + W.letterGap - W.ireupL }
}

const markLayer = (markScale) => (fg, accent) =>
  `<g transform="translate(${(-M.stemL * markScale).toFixed(2)} ${(W.baseline - M.foot * markScale).toFixed(2)}) scale(${markScale.toFixed(4)})">${innerOf(recolour(master('mark'), fg, accent))}</g>`

const drawn = (markup, fg, accent, shift) =>
  `<g ${recolour(PAINT, fg, accent)} transform="translate(${shift.toFixed(2)} 0)">${recolour(markup, fg, accent)}</g>`

// One candidate: a whole lockup, with the four things this round can move named
// explicitly so no row can quietly inherit one of them.
const variant = ({ id, label, short, note, air, uTop, dotCy, markScale = FROZEN_SCALE }) => {
  const { shift } = placed(markScale)
  const arrow = arrowAt(air, uTop)
  const letters = (fg, accent) =>
    [
      markLayer(markScale)(fg, accent),
      spell(['i', 'r', 'e', 'p'], { shift, fg, accent }),
      drawn(capU(uTop), fg, accent, shift),
      drawn(dotAt(dotCy), fg, accent, shift),
    ].join('\n  ')
  return {
    id,
    label,
    short: short ?? label,
    note,
    air,
    uTop,
    dotCy,
    markScale,
    shift,
    arrow,
    letters,
    arrowLayer: (accent) => arrow.draw(accent, shift),
    body: (fg, accent) => [letters(fg, accent), arrow.draw(accent, shift)].join('\n  '),
  }
}

// --- 11a: the three answers, applied one at a time ----------------------------
// In the order the card asked them, so each row shows one change against the one
// above it. The last row is what ships.
const PR = variant({
  id: 'a-pr',
  short: 'A · round 10',
  label: 'A · Round 10, on the PR now — 3 units, U short of the cap line',
  air: 3,
  uTop: W.ascTop,
  dotCy: DOT_ROUND10,
  note:
    'What you signed off with a note. The U stops\n' +
    '4 units below the F; the dot keeps 7 where the\n' +
    'arrow keeps 3.',
})

const CAP = variant({
  id: 'a-cap',
  short: "B · U to the cap line",
  label: "B · + the U's stems raised to the cap line — the arrow follows it up 4",
  air: 3,
  uTop: 0,
  dotCy: DOT_ROUND10,
  note:
    'The letter grows into the slot, so the arrow\n' +
    'has to rise with it to keep the same 3. This\n' +
    'is the row where "tighter" has not happened\n' +
    'yet and the whole accent is already higher.',
})

const TIGHT = variant({
  id: 'a-tight',
  short: 'C · gap to 2',
  label: 'C · + the gap taken to 2 — the arrow comes down 1',
  air: 2,
  uTop: 0,
  dotCy: DOT_ROUND10,
  note:
    'A quarter of the monoline weight. The slot is\n' +
    'the thing to look at here, not the number:\n' +
    'at the 96px minimum it is the renderer\n' +
    'deciding, not the drawing.',
})

const SHIP = variant({
  id: 'a-ship',
  short: 'D · + dot to 3. Ships.',
  label: "D · + the i's dot down to 3 as well. Ships.",
  air: 2,
  uTop: 0,
  dotCy: DOT_SHIPPED,
  note:
    'All three answers. The word now keeps one\n' +
    'accent clearance instead of two, and both\n' +
    'capitals top out on the same line.\n' +
    "The mark is the size it has been since round 6 —\n" +
    'see 11b.',
})

const ANSWERS = [PR, CAP, TIGHT, SHIP]

// The other reading of answer 2. "Bring the dot down to 3 too" was answered on
// a card where the arrow was at 3, and the same card moved the arrow to 2 — so
// "3" and "match the arrow" are no longer the same instruction. 3 is the
// literal answer and ships; this is the one that matches the number instead,
// drawn so the choice can be looked at rather than reasoned about.
const MATCH = variant({
  id: 'a-match',
  short: 'E · dot to 2 instead',
  label: "E · The dot at 2 as well — the arrow's new number",
  air: 2,
  uTop: 0,
  dotCy: DOT_MATCH,
  note:
    'Answer 2 read as "match the arrow" rather than\n' +
    'as the number 3. One unit tighter than what\n' +
    'ships.',
})

// --- 11b: what the dot does to the mark's size --------------------------------
// The question the third answer opens and cannot settle on its own.
const SHIP_DERIVED = variant({
  id: 'b-derived',
  short: 'B · mark re-solved',
  label: `B · The rule re-solved — mark at ${derivedScale(DOT_SHIPPED).toFixed(3)}x`,
  air: 2,
  uTop: 0,
  dotCy: DOT_SHIPPED,
  markScale: derivedScale(DOT_SHIPPED),
  note:
    'build.mjs as written: scale the mark until its\n' +
    'bar clears the dot by 3. The dot moved down, so\n' +
    'the rule returns a mark 8% smaller — and the\n' +
    'lockup aspect, the header, the clear-space unit\n' +
    'and the social card all move with it.',
})

const SIZES = [
  { ...SHIP, label: `A · Mark frozen at ${FROZEN_SCALE}x — round 6's size. Ships.`, note:
    'The mark stays the size it was signed off at.\n' +
    `Its bar now clears the dot by ${barAir(DOT_SHIPPED, FROZEN_SCALE).toFixed(0)} rather than 3,\n` +
    'which the rule allows: it is a floor the bar\n' +
    'has to clear, not a size to re-derive. Nothing\n' +
    'downstream of the lockup moves.' },
  SHIP_DERIVED,
  { ...PR, label: 'C · Round 10, for comparison', note: 'Dot at 7, mark at 1.5x, bar clearing the dot\nby 3 — the rule at its own minimum.' },
]

// --- diagnostics --------------------------------------------------------------
const framed = (opt, fg, accent) => {
  const body = opt.body(fg, accent)
  const box = contentBox(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="-80 -110 460 250">${body}</svg>`)
  const viewBox = `${box.x.toFixed(2)} ${box.y.toFixed(2)} ${box.w.toFixed(2)} ${box.h.toFixed(2)}`
  return {
    box,
    body,
    viewBox,
    svg: (bg) =>
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}">` +
      (bg ? `<rect x="${box.x.toFixed(2)}" y="${box.y.toFixed(2)}" width="${box.w.toFixed(2)}" height="${box.h.toFixed(2)}" fill="${bg}"/>` : '') +
      `${body}</svg>`,
  }
}

const MIN = 96 // the stated lockup minimum, in px wide
const REAL_WIDTHS = [320, 200, 128, MIN]
const TODAY = framed(PR, PAPER, EMBER)

console.log('--- the correction this round applies, re-measured off the raster ---')
console.log(
  `capital F ink top y=${F_TOP.toFixed(2)}; capital U as drawn y=${U_TOP.drawn.toFixed(2)}, ` +
    `stems raised to 0 y=${U_TOP.raised.toFixed(2)} -> the two capitals ` +
    `${Math.abs(F_TOP - U_TOP.raised) < 0.01 ? 'align exactly' : `still differ by ${(U_TOP.raised - F_TOP).toFixed(2)}`}`,
)
console.log(
  `i stem ink top y=${GEO.stemTop} by construction, y=${STEM_TOP.toFixed(2)} measured; the tittle keeps ` +
    `${dotAir(DOT.cy)} units of air at cy=${DOT.cy} and ${dotAir(DOT_SHIPPED)} at cy=${DOT_SHIPPED}, which is what ships ` +
    `(measured ${measuredDotAir(DOT_SHIPPED).toFixed(2)} — see the note on measurement bias above)`,
)
console.log(
  `the arrow glyph in the master measures ${ARROW_BOX.w.toFixed(1)}x${ARROW_BOX.h.toFixed(1)} units, lowest ink ` +
    `y=${(ARROW_BOX.y + ARROW_BOX.h).toFixed(2)} measured against y=${GEO.arrowBottom} by construction`,
)
console.log(
  `placing it over today's U at today's gap gives dy=${PR.arrow.dy} — the master unchanged, which is the check; ` +
    `shipping ${SHIP.air} units over a raised U moves it ${SHIP.arrow.dy > 0 ? 'down' : 'up'} ` +
    `${Math.abs(SHIP.arrow.dy)} whole units, to lowest ink y=${GEO.arrowBottom + SHIP.arrow.dy}`,
)
console.log(
  `\nthe mark's bar over the dot: ${barAir(DOT.cy, FROZEN_SCALE).toFixed(2)} units today, ` +
    `${barAir(DOT_SHIPPED, FROZEN_SCALE).toFixed(2)} once the dot drops, at the frozen ${FROZEN_SCALE}x. ` +
    `Re-solving the rule for the new dot gives ${derivedScale(DOT_SHIPPED).toFixed(4)}x ` +
    `(${((derivedScale(DOT_SHIPPED) / FROZEN_SCALE - 1) * 100).toFixed(1)}%), which is 11b.`,
)
console.log(
  `today's lockup envelope: ${TODAY.box.w.toFixed(1)} x ${TODAY.box.h.toFixed(1)} units, ` +
    `${(TODAY.box.w / TODAY.box.h).toFixed(2)}:1, top at y=${TODAY.box.y.toFixed(1)}\n`,
)

// The slot between the two shapes, in device pixels, at every width the lockup
// is really drawn at — plus the air averaged over the slot's run, because a
// parallel slot and a pinching one do not read the same at the same minimum.
const report = (options) =>
  console.table(
    options.map((opt) => {
      const row = framed(opt, PAPER, EMBER)
      const arrowLayer = opt.arrowLayer(EMBER)
      const letterLayer = opt.letters(PAPER, EMBER)
      const slot = slotRun({ viewBox: row.viewBox, boxWidth: row.box.w, upper: arrowLayer, lower: letterLayer, limit: W.stroke })
      const unit = MIN / row.box.w
      return {
        id: opt.id,
        'air (units)': opt.air.toFixed(1),
        'U top': opt.uTop === W.ascTop ? 'as drawn' : 'raised',
        'dot cy': opt.dotCy,
        'mark': `${opt.markScale.toFixed(3)}x`,
        aspect: `${(row.box.w / row.box.h).toFixed(2)}:1`,
        'env vs today': (row.box.h - TODAY.box.h).toFixed(1),
        'slot runs': `${slot.run.toFixed(1)}u`,
        'mean air': slot.mean.toFixed(1),
        '320px': inkGap(row.viewBox, arrowLayer, letterLayer, 320).toFixed(2),
        '200px': inkGap(row.viewBox, arrowLayer, letterLayer, 200).toFixed(2),
        '128px': inkGap(row.viewBox, arrowLayer, letterLayer, 128).toFixed(2),
        '96px': inkGap(row.viewBox, arrowLayer, letterLayer, MIN).toFixed(2),
        'x-height at 96px': ((W.baseline - W.xTop) * unit).toFixed(1),
      }
    }),
  )

console.log('11a — the three answers, one at a time (arrow against the U):')
report(ANSWERS)
console.log('\n11b — what the dot does to the mark (arrow against the U, for reference):')
report(SIZES)

// The dot's own slot, measured the same way, so "both accents keep 3" is a
// measurement and not a claim. Also whether the two slots are the same *shape*:
// the arrow's tail is flat over a flat cap, so its mean air is its minimum; the
// dot is a circle over a flat stem, so its slot pinches and opens out.
const dotSlotOf = (opt) => {
  const row = framed(opt, PAPER, EMBER)
  const dotLayer = drawn(dotAt(opt.dotCy), PAPER, EMBER, opt.shift)
  const stemLayer = spell(['i'], { shift: opt.shift, fg: PAPER, accent: EMBER })
  const slot = slotRun({ viewBox: row.viewBox, boxWidth: row.box.w, upper: dotLayer, lower: stemLayer, limit: W.stroke * 2 })
  return {
    id: opt.id,
    'dot cy': opt.dotCy,
    'air (units)': dotAir(opt.dotCy).toFixed(2),
    'slot runs': `${slot.run.toFixed(1)}u`,
    tightest: slot.min.toFixed(1),
    'mean air': slot.mean.toFixed(1),
    ...Object.fromEntries(REAL_WIDTHS.map((w) => [`${w}px`, inkGap(row.viewBox, dotLayer, stemLayer, w).toFixed(2)])),
  }
}
console.log("\nthe i's own accent, measured the same way — the second half of answer 2:")
console.table([dotSlotOf(PR), dotSlotOf(SHIP), dotSlotOf(MATCH)])
// Why the two accents are not given the same number. The dot is a circle over a
// flat stem, so its slot pinches at one point and opens out on both sides; the
// arrow's tail is flat over a flat cap, so its slot is parallel and its mean air
// is its minimum. Equal minimums would not read as equal air — which is the
// measured reason the literal answer, 3 against the arrow's 2, is also the one
// that looks right.
console.log(
  `the dot's slot at cy=${DOT_SHIPPED} pinches to ${dotSlotOf(SHIP).tightest} and averages ${dotSlotOf(SHIP)['mean air']} ` +
    `over its run; the arrow's is parallel, so its mean air is its minimum.`,
)

// --- the sheets ---------------------------------------------------------------
const FONT = 'DejaVu Sans, sans-serif'
const PAD = 28
const ROW_GAP = 34
const NOTE_COL = 340

const text = (x, y, body, { size = 15, fill = PAPER, weight = 'bold' } = {}) =>
  `<text x="${x}" y="${y}" font-family="${FONT}" font-size="${size}" font-weight="${weight}" fill="${fill}">${body}</text>`

const sheetOf = ({ options, fg, accent, bg, muted, caption }) => {
  const rows = options.map((opt) => framed(opt, fg, accent))
  const width = PAD * 2 + REAL_WIDTHS.reduce((sum, w) => sum + w + 28, 0) + NOTE_COL
  let y = PAD + 30
  const blocks = options
    .map((opt, index) => {
      const row = rows[index]
      const rowH = (REAL_WIDTHS[0] * row.box.h) / row.box.w
      const top = y
      let x = PAD
      const tiles = REAL_WIDTHS.map((w) => {
        const h = (w * row.box.h) / row.box.w
        const tile = `<svg viewBox="${row.viewBox}" x="${x}" y="${(top + 22 + (rowH - h) / 2).toFixed(2)}" width="${w}" height="${h.toFixed(2)}">${row.body}</svg>`
        x += w + 28
        return tile
      }).join('')
      const notes = opt.note
        .split('\n')
        .map((line, i) => text(x + 6, top + 36 + i * 17, line, { size: 13, fill: muted, weight: 'normal' }))
        .join('')
      y = top + 22 + rowH + ROW_GAP
      return `${text(PAD, top + 12, opt.label, { fill: fg })}${tiles}${notes}`
    })
    .join('')

  const height = y + 30
  let labelX = PAD
  const scale = REAL_WIDTHS.map((w) => {
    const label = text(labelX, PAD + 10, `${w}px wide`, { size: 12, fill: muted, weight: 'normal' })
    labelX += w + 28
    return label
  }).join('')

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
  <rect width="${width}" height="${height}" fill="${bg}"/>
  ${scale}
  ${text(labelX + 6, PAD + 10, 'actual size, left to right', { size: 12, fill: muted, weight: 'normal' })}
  ${blocks}
  ${text(PAD, height - 12, caption, { size: 14, fill: muted, weight: 'normal' })}
</svg>`
}

// A sub-pixel slot cannot be argued about from a sheet, so: render at the width
// in question, read the renderer's own pixels back, and redraw them as one
// square per pixel. Every square is a real pixel at its real colour.
const zoomSheet = ({ options, fg, accent, bg, muted, caption, width: px, scale = 5 }) => {
  const rows = options.map((opt) => framed(opt, fg, accent))
  let y = PAD + 16
  const blocks = options
    .map((opt, index) => {
      const row = rows[index]
      const h = Math.round((px * row.box.h) / row.box.w)
      const top = y
      y = top + 24 + h * scale + 30
      return `${text(PAD, top + 12, opt.label, { fill: fg })}
      <g transform="translate(${PAD} ${top + 24})">${pixelMap(row.svg(bg), px, scale)}</g>`
    })
    .join('')
  const width = PAD * 2 + px * scale + 40
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${y + 24}" width="${width}" height="${y + 24}">
  <rect width="${width}" height="${y + 24}" fill="${bg}"/>
  ${blocks}
  ${text(PAD, y + 8, caption, { size: 13, fill: muted, weight: 'normal' })}
</svg>`
}

// The U and its accent on their own, big. The whole lockup at 320 gives about
// 11px to the thing being decided; this crops to it so the rows can be compared
// as drawings. The dot and its stem get the same treatment, because answer 2 is
// about whether the two slots match.
const cropSheet = ({ options, crop, fg, accent, bg, muted, caption, layers, tile = 240, foot }) => {
  // Every crop is the same window on a different drawing, so one tile height
  // serves the row — taken from the first rather than assumed.
  const first = crop(options[0])
  const tileH = (tile * first.h) / first.w
  let x = PAD
  const tiles = options
    .map((opt) => {
      const body = layers(opt, fg, accent)
      const window = crop(opt)
      const out =
        `<svg viewBox="${window.x.toFixed(2)} ${window.y.toFixed(2)} ${window.w} ${window.h}" x="${x}" y="${PAD + 34}" width="${tile}" height="${tileH.toFixed(2)}">${body}</svg>` +
        text(x, PAD + 22, opt.short, { size: 14, fill: fg }) +
        text(x, PAD + 48 + tileH, (foot ?? ((o) => `${o.air.toFixed(1)} units of air`))(opt), { size: 13, fill: muted, weight: 'normal' })
      x += tile + 20
      return out
    })
    .join('')
  const width = PAD * 2 + options.length * (tile + 20)
  const height = PAD * 2 + 60 + tileH + 30
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
  <rect width="${width}" height="${height}" fill="${bg}"/>
  ${tiles}
  ${text(PAD, height - 14, caption, { size: 14, fill: muted, weight: 'normal' })}
</svg>`
}

// 11c: the two capitals, which is the only place the third answer is visible.
// The lockup's F is the mark, so this shows the word-alone variant — the one
// drawing in the identity that sets a capital F next to a capital U. A rule is
// drawn across the word at the F's ink top; whether the U reaches it is the
// whole question, and a guide makes that readable rather than arguable.
const capSheet = ({ fg, accent, bg, muted }) => {
  const rows = [
    {
      label: 'A · Round 10 — the U stops 4 units short of the F',
      uTop: W.ascTop,
      dotCy: DOT_ROUND10,
      air: 3,
      note: 'Both stems are butt-capped, so they terminate on the ascender line instead of straddling it.',
    },
    {
      label: 'B · The stems run to the cap line. Ships.',
      uTop: 0,
      dotCy: DOT_SHIPPED,
      air: 2,
      note: "The F's cap bar is a horizontal stroke centred on the same line, so its ink already tops out there.",
    },
  ]
  const WIDE = 760
  const built = rows.map((row) => {
    const arrow = arrowAt(row.air, row.uTop === W.ascTop ? U_TOP.drawn : U_TOP.raised)
    const body = [
      spell(['F', 'i', 'r', 'e', 'p'], { fg, accent }),
      drawn(capU(row.uTop), fg, accent, 0),
      drawn(dotAt(row.dotCy), fg, accent, 0),
      arrow.draw(accent, 0),
    ].join('\n  ')
    const box = contentBox(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="-20 -60 260 160">${body}</svg>`)
    return { ...row, body, box }
  })
  const box = {
    x: Math.min(...built.map((r) => r.box.x)) - 4,
    y: Math.min(...built.map((r) => r.box.y)) - 4,
    w: Math.max(...built.map((r) => r.box.x + r.box.w)) - Math.min(...built.map((r) => r.box.x)) + 8,
    h: Math.max(...built.map((r) => r.box.y + r.box.h)) - Math.min(...built.map((r) => r.box.y)) + 8,
  }
  const unit = WIDE / box.w
  const rowH = box.h * unit
  let y = PAD + 24
  const blocks = built
    .map((row) => {
      const top = y
      // 1.2 device pixels, whatever the tile's scale works out to.
      const hair = 1.2 / unit
      const guide =
        `<line x1="${box.x.toFixed(2)}" y1="${F_TOP.toFixed(3)}" x2="${(box.x + box.w).toFixed(2)}" y2="${F_TOP.toFixed(3)}" stroke="${accent}" stroke-width="${hair.toFixed(3)}" stroke-dasharray="${(hair * 3).toFixed(2)} ${(hair * 3).toFixed(2)}" opacity="0.8"/>`
      const tile = `<svg viewBox="${box.x.toFixed(2)} ${box.y.toFixed(2)} ${box.w.toFixed(2)} ${box.h.toFixed(2)}" x="${PAD}" y="${top + 20}" width="${WIDE}" height="${rowH.toFixed(2)}">${row.body}${guide}</svg>`
      y = top + 20 + rowH + 46
      return (
        text(PAD, top + 8, row.label, { fill: fg }) +
        tile +
        text(PAD, top + 20 + rowH + 20, row.note, { size: 13, fill: muted, weight: 'normal' })
      )
    })
    .join('')
  const width = PAD * 2 + WIDE
  const height = y + 16
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
  <rect width="${width}" height="${height}" fill="${bg}"/>
  ${blocks}
  ${text(PAD, height - 8, `Round 11c — the word-alone variant, ${WIDE}px wide   ·   the dashed rule is the capital F's own ink top, y=${F_TOP.toFixed(2)}`, { size: 13, fill: muted, weight: 'normal' })}
</svg>`
}

// --- write --------------------------------------------------------------------
const out = join(root, 'align')
mkdirSync(out, { recursive: true })

const DARK = { fg: PAPER, accent: EMBER, bg: INK, muted: MUTED }
const LIGHT = { fg: INK, accent: EMBER_DEEP, bg: PAPER, muted: '#5A6782' }

const emit = (name, source) => {
  const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const file = join(out, `${name}.png`)
  writeFileSync(file, renderPng(source, width))
  console.log(file)
}

emit(
  'answers-dark',
  sheetOf({
    options: ANSWERS,
    ...DARK,
    caption: `Round 11a — the three answers applied one at a time   ·   on ink, ember ${EMBER}   ·   each row is one change against the row above it`,
  }),
)
emit(
  'answers-light',
  sheetOf({
    options: ANSWERS,
    ...LIGHT,
    caption: `Round 11a — the three answers applied one at a time   ·   on paper, ember-deep ${EMBER_DEEP}   ·   each row is one change against the row above it`,
  }),
)
emit(
  'size-dark',
  sheetOf({
    options: SIZES,
    ...DARK,
    caption: "Round 11b — what the dot does to the mark's size   ·   on ink   ·   A and B differ only in the mark; the word is identical",
  }),
)
emit(
  'size-light',
  sheetOf({
    options: SIZES,
    ...LIGHT,
    caption: "Round 11b — what the dot does to the mark's size   ·   on paper   ·   A and B differ only in the mark; the word is identical",
  }),
)

// Cropped to the U and its accent — the same window on every tile, anchored to
// the construction rather than to each option's own letter. An earlier cut
// anchored it to the U's measured top, which moved the window with the thing it
// was supposed to be showing: the raised U came out looking identical to the
// drawn one. A fixed window is the whole point of the sheet.
const U_CROP = (opt) => ({ x: opt.shift + 120 - W.stroke / 2 - 16, y: -36, w: 76, h: 98 })
const U_LAYERS = (opt, fg, accent) =>
  [drawn(capU(opt.uTop), fg, accent, opt.shift), opt.arrow.draw(accent, opt.shift)].join('')

emit(
  'detail-dark',
  cropSheet({
    options: ANSWERS,
    crop: U_CROP,
    layers: U_LAYERS,
    ...DARK,
    caption: 'Round 11a, cropped to the U and its accent   ·   on ink   ·   magnified ~3.2x from the 320px tile, where the decision is 11px tall',
  }),
)
emit(
  'detail-light',
  cropSheet({
    options: ANSWERS,
    crop: U_CROP,
    layers: U_LAYERS,
    ...LIGHT,
    caption: 'Round 11a, cropped to the U and its accent   ·   on paper   ·   magnified ~3.2x from the 320px tile, where the decision is 11px tall',
  }),
)

// Cropped to the i and its dot — the other half of the same answer, at the same
// magnification, so the two accents can be compared as drawings.
emit(
  'accents-dark',
  cropSheet({
    options: [PR, SHIP, MATCH],
    crop: (opt) => ({ x: opt.shift + DOT.cx - 24, y: -2, w: 48, h: 62 }),
    layers: (opt, fg, accent) =>
      [spell(['i'], { shift: opt.shift, fg, accent }), drawn(dotAt(opt.dotCy), fg, accent, opt.shift)].join(''),
    foot: (opt) => `${dotAir(opt.dotCy).toFixed(1)} units of air`,
    ...DARK,
    caption: "Round 11, cropped to the i and its dot   ·   on ink   ·   the same window on all three; the arrow's own gap is sheet 11a",
  }),
)

emit('caps-dark', capSheet(DARK))
emit('caps-light', capSheet(LIGHT))

emit(
  'zoom-96',
  zoomSheet({
    options: [...ANSWERS, MATCH],
    ...DARK,
    width: MIN,
    scale: 5,
    caption: `Round 11a at ${MIN}px wide — the stated lockup minimum — magnified 5x, real pixels, no resampling`,
  }),
)
emit(
  'zoom-200',
  zoomSheet({
    options: ANSWERS,
    ...DARK,
    width: 200,
    scale: 4,
    caption: 'Round 11a at 200px wide — the site header size — magnified 4x, real pixels, no resampling',
  }),
)
emit(
  'zoom-size',
  zoomSheet({
    options: SIZES,
    ...DARK,
    width: MIN,
    scale: 5,
    caption: `Round 11b at ${MIN}px wide — magnified 5x, real pixels, no resampling`,
  }),
)

for (const opt of [...ANSWERS, MATCH, SHIP_DERIVED]) {
  const row = framed(opt, PAPER, EMBER)
  for (const w of [MIN, 960]) {
    const file = join(out, `${opt.id}-${w}.png`)
    writeFileSync(file, renderPng(row.svg(INK), w))
    console.log(file)
  }
}

// --- what goes into the master ------------------------------------------------
// Printed so the master is transcribed rather than worked out twice. The arrow
// is the approved glyph moved vertically, so what matters is one offset, not a
// new path: every y in the arrow's two paths shifts by the same amount, which
// keeps the drawing bit-identical in shape.
//
// This is round 11's transcription, and round 12 has since moved the glyph one
// unit right, so the x values are put back where round 11 left them — otherwise
// this would print an instruction that silently undoes a later round.
console.log(`\n--- what goes into logo/wordmark.svg ---`)
console.log(`U:      stems to y=0       <path d="M120 0 V35 A15 15 0 0 0 150 35 V0"/>`)
console.log(`tittle: cy ${DOT.cy} -> ${DOT_SHIPPED}         <circle cx="${DOT.cx}" cy="${DOT_SHIPPED}" r="${DOT.r}" fill="#FFFFFF" stroke="none"/>`)
console.log(
  `arrow:  every y ${SHIP.arrow.dy > 0 ? '+' : '-'} ${Math.abs(SHIP.arrow.dy)}, exactly — the approved glyph moved by a whole ` +
    `number of units, so its shape is untouched. ${SHIP.air} units of air over the raised U:`,
)
console.log(
  ARROW.split('\n')
    .map((line) =>
      line.replace(/([ML])([-\d.]+) ([-\d.]+)/g, (_, cmd, x, y) => `${cmd}${Number(x) + ARROW_DX} ${Number(y) + SHIP.arrow.dy}`)
        .replace(/([QL]) ?([-\d.]+) ([-\d.]+) ([-\d.]+) ([-\d.]+)/g, (_, cmd, x1, y1, x2, y2) => `${cmd}${Number(x1) + ARROW_DX} ${Number(y1) + SHIP.arrow.dy} ${Number(x2) + ARROW_DX} ${Number(y2) + SHIP.arrow.dy}`)
        .trim(),
    )
    .map((line) => `  ${line}`)
    .join('\n'),
)
