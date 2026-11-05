#!/usr/bin/env node
// Round thirteen. Two answers came back on round 12's card:
//
//   lockup   -> "Back to round 7's centring (row C)"
//   right end-> "Draw it" (both ends flush, which costs a redraw of the chord)
//
//   node scripts/flush.mjs   ->  brand/flush/*.png
//
// FIRST, THE DEFECT, because it is why those two answers point opposite ways.
//
// scripts/edge.mjs took the arrow's current position from a constant — 115, the
// x the tail sat on while round 12 was being drawn — and built each option as an
// offset from it. The moment the master was edited to round 12's answer, that
// constant was a unit stale, and every row on those sheets rendered one unit to
// the right of its own caption:
//
//   row A, captioned "edges flush, ships"  ->  drawn 1 unit RIGHT of the U's edge
//   row C, captioned "centred, 1 unit left" ->  drawn exactly ON the U's edge
//
// So the row that lines up in the picture is the row labelled C, and C is also
// what is on the PR. Read by its caption the answer says move the accent a unit
// left; read by the picture it says keep what is shipped. Both readings are
// honest and they disagree, and that is this script's fault rather than the
// reader's.
//
// The fix is structural and it is the rule for anything that follows: an option
// here is BUILT FROM ITS RULE — chord, rise, head, and where the tail lands —
// and never from "the master, plus an offset". A sheet composed out of the live
// drawing is only correct until the drawing moves, which is exactly when it is
// being looked at.
//
// SECOND, THE REDRAW. "Draw it" was answered against this: the arrow is 39.4
// wide against the U's 38, because the head's corner overhangs the chord, so the
// two cannot be flush at both ends without the drawing losing that 1.4. There
// are only two places it can come off — the chord, or the head — and both are
// drawn below as D and E. Two other levers were measured and neither reaches
// flush at all; see the scan this prints.
//
// AND THE ANSWERS STOP DISAGREEING. An object the same width as the letter is
// centred on it exactly when its edges line up, so D is round 7's centring rule
// and round 12's flush-left rule and flush-right, all at once. There is no
// horizontal placement question left after this round, whichever way the answer
// above is read.
//
// The mark is frozen — signed off in round 5, untouched since. Nothing here
// opens mark.svg.
import { renderPng, contentBox, pixelMap } from './render.mjs'
import { inkGap, slotRun } from './measure.mjs'
import { master, innerOf, recolour, spell, GLYPHS, W } from './word.mjs'
import { readFileSync, mkdirSync, writeFileSync, existsSync } from 'node:fs'
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

const MIN = 96 // the stated lockup minimum
const REAL_WIDTHS = [320, 200, 128, MIN]

// `--draft` lets this run while the master is still being transcribed from it.
// Without it the script asserts that the master draws the shipped option, so
// `npm run flush` is a standing check that the file and this script agree.
const DRAFT = process.argv.includes('--draft')

const PAINT = `fill="none" stroke="#FFFFFF" stroke-width="${W.stroke}" stroke-linecap="butt" stroke-linejoin="round"`
const alone = (markup) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="-60 -90 340 200"><g ${PAINT}>${markup}</g></svg>`
const inkOf = (markup) => contentBox(alone(markup), 2048)

// --- the letter ---------------------------------------------------------------
// Both of the U's stems are butt-capped verticals centred on x=120 and x=150, so
// each presents one flat edge and each edge is exactly half a stroke outside its
// stem. The two numbers this whole round turns on are therefore construction and
// not measurement: 116 and 154, exactly.
//
// They are measured as well, and the two are printed side by side below. The
// raster is the check; it is not the source. Round 13's first cut had it the
// other way round and that is what went wrong — see `shaped` below.
const U_LEFT = 120 - W.stroke / 2
const U_RIGHT = 150 + W.stroke / 2
const U_WIDTH = U_RIGHT - U_LEFT
const U_BOX = inkOf(GLYPHS.get('U'))

// --- the arrow, from its rules -------------------------------------------------
// Round 7's construction, rebuilt here rather than lifted: tail at the left on a
// horizontal, a quadratic bowing like a growth curve, and a solid head square to
// the end tangent. Everything that is not being asked about is held fixed.
const BOW = 0.72 // control point level with the tail, 72% along
const RISE = 0.43 // the chord's rise as a fraction of its run
const HEAD = { half: 1.4, long: 1.7 } // in shafts: 2.8 wide, 1.7 long
const CHORD = 150 - 120 // 30 — the U's stem pitch, which is where the number came from
const BOTTOM = -2 // the accent's lowest ink: 2 clear of the U's top. Round 11's answer.

const one = (n) => Number(n.toFixed(1))

// The arrow, and where its ink actually falls — both derived from the
// coordinates that get written into the file. Nothing here samples a raster.
//
// The head is built from the arc's DRAWN coordinates, not from the ideal ones
// they were rounded off. The master rounds what it writes, so deriving from the
// ideal leaves the head sitting a fraction off the end of the shaft it caps. At
// round 7's numbers that error was 0.1 units and invisible; at this round's
// chord the neck lands mid-unit and it is 0.5, which is a third of what the
// whole round is about.
const arrowAt = ({ left, run, rise, head = {} }) => {
  const { w: headW = 1, l: headL = 1 } = head
  const tailY = BOTTOM - W.stroke / 2
  const a = { x: one(left), y: one(tailY) }
  const ctrl = { x: one(left + run * BOW), y: one(tailY) }
  const neck = { x: one(left + run), y: one(tailY - rise) }
  const span = { x: neck.x - ctrl.x, y: neck.y - ctrl.y }
  const len = Math.hypot(span.x, span.y)
  const u = { x: span.x / len, y: span.y / len }
  const n = { x: -u.y, y: u.x }
  const half = W.stroke * HEAD.half * headW
  const long = W.stroke * HEAD.long * headL
  // The head's three corners, rounded the way they are written out — so these
  // are the drawing's real corners and not the ideal ones they came from.
  const corners = [
    { x: neck.x + n.x * half, y: neck.y + n.y * half },
    { x: neck.x + u.x * long, y: neck.y + u.y * long },
    { x: neck.x - n.x * half, y: neck.y - n.y * half },
  ].map((p) => ({ x: one(p.x), y: one(p.y) }))

  // The stroked shaft's own envelope: the quadratic offset by half a stroke on
  // either side. Sampled densely in closed form rather than rasterised. The head
  // covers it in this drawing, but that is a fact about round 7's proportions
  // and not something to assume of every option on the sheet.
  const edge = []
  for (let i = 0; i <= 256; i += 1) {
    const t = i / 256
    const px = (1 - t) ** 2 * a.x + 2 * (1 - t) * t * ctrl.x + t ** 2 * neck.x
    const py = (1 - t) ** 2 * a.y + 2 * (1 - t) * t * ctrl.y + t ** 2 * neck.y
    const dx = 2 * (1 - t) * (ctrl.x - a.x) + 2 * t * (neck.x - ctrl.x)
    const dy = 2 * (1 - t) * (ctrl.y - a.y) + 2 * t * (neck.y - ctrl.y)
    const d = Math.hypot(dx, dy) || 1
    const ox = (-dy / d) * (W.stroke / 2)
    const oy = (dx / d) * (W.stroke / 2)
    edge.push({ x: px + ox, y: py + oy }, { x: px - ox, y: py - oy })
  }

  const xs = [...corners, ...edge].map((p) => p.x)
  const at = (p) => `${p.x} ${p.y}`
  return {
    markup: [
      `<path d="M${a.x} ${a.y} Q${ctrl.x} ${ctrl.y} ${neck.x} ${neck.y}" stroke="#FF5A2B" stroke-width="${W.stroke}" stroke-linecap="butt"/>`,
      `<path d="M${at(corners[0])} L${at(corners[1])} L${at(corners[2])} Z" fill="#FF5A2B" stroke="none"/>`,
    ].join('\n    '),
    // The tail is a butt cap on a horizontal, so the start coordinate IS the
    // leftmost ink — no cap rounds past it.
    left: Math.min(a.x, ...xs),
    right: Math.max(...xs),
  }
}

const arrowPaths = (spec) => arrowAt(spec).markup

// An arrow of the given shape, placed by the given rule. `place` takes the
// shape's own width and returns where its left ink goes, so a rule is written
// once and holds whatever the shape turns out to be.
//
// THE PLACEMENT IS COMPUTED, NOT SAMPLED, and round 13 learned that the hard
// way. The first cut probed the shape by rendering it and reading `contentBox`,
// which rounds outward to whole sample pixels — 0.166 units at this viewBox and
// sample size, sixteen times coarser than the tenth of a unit the result was
// then snapped to. The tail came out at 116.1 against the U's 116 and the
// shipped lockup was a tenth of a unit off flush, in the round whose entire
// subject is being flush. The built-asset check at the foot of this file is what
// caught it.
//
// So: the geometry is closed form, and the raster is kept as the independent
// check on it rather than as its source. Measuring is still the right instinct —
// measuring at a resolution coarser than the claim is the mistake.
const shaped = ({ run, rise = run * RISE, head = {}, place }) => {
  const probe = arrowAt({ left: 0, run, rise, head })
  const left = place(probe.right - probe.left)
  const drawn = arrowAt({ left: left - probe.left, run, rise, head })
  const { w: headW = 1 } = head
  return {
    markup: drawn.markup,
    box: inkOf(drawn.markup), // the raster's opinion, for the table and the sheets
    left: drawn.left,
    right: drawn.right,
    run,
    rise,
    head,
    headW,
    width: drawn.right - drawn.left,
  }
}

const FLUSH_LEFT = () => U_LEFT
const CENTRED = (width) => U_LEFT + (U_WIDTH - width) / 2

// The master carries numbers to a tenth so it stays a drawing a person can read
// and correct. The U's own edges are whole units, so a rule that lands on one
// needs no snapping — but the centred rule lands mid-unit, and the file should
// not inherit a long decimal from it.
const snapped = (rule) => (width) => one(rule(width))

// The chord that makes the arrow exactly as wide as the letter.
//
// Solved rather than subtracted. "Take 1.5 off the 30" is right to within the
// rounding, and the rounding is the whole subject: every coordinate goes into
// the file at a tenth, so the head's corner moves in steps and the run that
// lands it on 154 is not simply 28.5. Scanned at a hundredth and the best is
// kept, which also reports honestly when no run lands it exactly.
const fittingChord = (head = {}) => {
  let best = null
  for (let run = CHORD - 3; run <= CHORD + 0.5; run += 0.01) {
    const probe = arrowAt({ left: 0, run, rise: run * RISE, head })
    const drawn = arrowAt({ left: U_LEFT - probe.left, run, rise: run * RISE, head })
    const miss = Math.abs(drawn.right - U_RIGHT)
    if (!best || miss < best.miss - 1e-9) best = { run: Number(run.toFixed(2)), miss }
  }
  return best
}

// --- the options ---------------------------------------------------------------
const FITTED = fittingChord()
const FITTED_HEAD = { w: 0.85, l: 0.85 }

// Round 7's arrow where round 12 put it, and the number every caption on this
// sheet turns on. Computed once and quoted from, rather than typed into four
// labels — the first draft said "1.5" in all of them because that is what the
// raster probe had reported, and it is 1.40.
const ROUND7 = shaped({ run: CHORD, place: FLUSH_LEFT })
const OVER = (ROUND7.width - U_WIDTH).toFixed(1)
console.log(
  `the chord that makes the arrow exactly as wide as the U: ${FITTED.run} ` +
    `(${(CHORD - FITTED.run).toFixed(2)} off the stem pitch), landing the head's corner ` +
    `${FITTED.miss < 0.005 ? 'exactly on' : `${FITTED.miss.toFixed(3)} from`} x=${U_RIGHT}.\n`,
)

const OPTIONS = [
  {
    id: 'a-today',
    label: 'A · Chord 30, flush on the left — what is on the PR today',
    short: 'A · Today',
    note:
      `Round 12. The accent starts where the letter starts and all ${OVER}\n` +
      "units of the head's overhang hang off the right. It is the row\n" +
      'that looked like "C" on round 12\'s sheet.',
    arrow: ROUND7,
  },
  {
    id: 'c-centred',
    label: "C · Chord 30, the arrow's box centred on the U's — round 7's rule",
    short: 'C · Centred',
    note:
      'The answer read by its caption. Balanced as an object: 1 unit\n' +
      'outside the letter on the left, 0.5 on the right. Neither edge\n' +
      'lines up with anything.',
    arrow: shaped({ run: CHORD, place: snapped(CENTRED) }),
  },
  {
    id: 'd-chord',
    label: `D · Both ends flush — the chord gives up the ${OVER}. Ships.`,
    short: `D · Chord ${FITTED.run}`,
    note:
      `The chord shortens past the head's ${OVER} of overhang, ${CHORD} to\n` +
      `${FITTED.run}, which is the run that lands the corner ON the U's edge\n` +
      'rather than near it. Same weight, same head, same bow.',
    arrow: shaped({ run: FITTED.run, place: FLUSH_LEFT }),
  },
  {
    id: 'e-head',
    label: `E · Both ends flush — the head gives up the ${OVER} instead`,
    short: 'E · Head 2.4',
    note:
      `The chord keeps its stem-to-stem ${CHORD} and the head comes down to\n` +
      '2.4 shafts wide. That is under the 2.5 this identity wrote down\n' +
      'as the floor where a head starts reading as a comma.',
    arrow: shaped({ run: CHORD, head: FITTED_HEAD, place: FLUSH_LEFT }),
  },
]

const SHIP = OPTIONS[2]

// --- the lockup -----------------------------------------------------------------
// Composed the way build.mjs composes it, so a row on these sheets is the asset
// and not an approximation of it. The mark's scale is the one that was signed
// off; nothing in this round touches it.
const M = { foot: 60, armEnd: 28, stemL: 6 }
const SCALE = 1.5 // round 6, signed off

const markX = (u) => (u - M.stemL) * SCALE
const SHIFT = markX(M.armEnd) + W.letterGap - W.ireupL

const markLayer = (fg, accent) =>
  `<g transform="translate(${(-M.stemL * SCALE).toFixed(2)} ${(W.baseline - M.foot * SCALE).toFixed(2)}) scale(${SCALE.toFixed(4)})">${innerOf(recolour(master('mark'), fg, accent))}</g>`

const lettersOf = (fg, accent) =>
  [markLayer(fg, accent), spell(['i', 'r', 'e', 'U', 'p', 'tittle'], { shift: SHIFT, fg, accent })].join('\n  ')

// The arrow paints itself — both its paths name the ember — but the group is
// painted anyway so a future glyph that inherits cannot come out the wrong
// colour on the light sheet.
const arrowLayerOf = (opt, accent, fg = PAPER) =>
  `<g ${recolour(PAINT, fg, accent)} transform="translate(${SHIFT.toFixed(2)} 0)">${recolour(opt.arrow.markup, fg, accent)}</g>`

const uLayerOf = (fg, accent) => spell(['U'], { shift: SHIFT, fg, accent })

const bodyOf = (opt, fg, accent) => [lettersOf(fg, accent), arrowLayerOf(opt, accent, fg)].join('\n  ')

const PADDING = 8
const framed = (opt, fg, accent) => {
  const body = bodyOf(opt, fg, accent)
  const box = contentBox(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="-40 -80 320 200">${body}</svg>`)
  const viewBox = `${(box.x - PADDING).toFixed(2)} ${(box.y - PADDING).toFixed(2)} ${(box.w + PADDING * 2).toFixed(2)} ${(box.h + PADDING * 2).toFixed(2)}`
  return {
    box,
    viewBox,
    body,
    svg: (bg) =>
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}"><rect x="${(box.x - PADDING).toFixed(2)}" y="${(box.y - PADDING).toFixed(2)}" width="${(box.w + PADDING * 2).toFixed(2)}" height="${(box.h + PADDING * 2).toFixed(2)}" fill="${bg}"/>${body}</svg>`,
  }
}

const TODAY = framed(OPTIONS[0], PAPER, EMBER)

// --- measurement ------------------------------------------------------------------
console.log(
  `the U's ink: x ${U_LEFT} to ${U_RIGHT}, ${U_WIDTH} wide, by construction — the stems are butt caps on 120 and 150. ` +
    `The raster agrees to the sample grid: ${U_BOX.x.toFixed(2)} to ${(U_BOX.x + U_BOX.w).toFixed(2)}.\n` +
    `Round 7's arrow is ${OPTIONS[0].arrow.width.toFixed(2)} wide, ` +
    `${(OPTIONS[0].arrow.width - U_WIDTH).toFixed(2)} more — so one of its two ends has to miss, ` +
    'unless the drawing gives that up.\n',
)

const rowOf = (opt) => {
  const row = framed(opt, PAPER, EMBER)
  const arrow = arrowLayerOf(opt, EMBER)
  const letters = lettersOf(PAPER, EMBER)
  // Computed from the drawn coordinates. The raster is checked against these
  // below rather than asked for them.
  const left = opt.arrow.left - U_LEFT
  const right = opt.arrow.right - U_RIGHT
  const slot = slotRun({ viewBox: row.viewBox, boxWidth: row.box.w, upper: arrow, lower: letters, limit: W.stroke })
  return { row, arrow, letters, left, right, slot }
}

const edge = (v) => (Math.abs(v) < 0.05 ? 'flush' : `${v > 0 ? '+' : ''}${v.toFixed(1)}u`)

console.table(
  OPTIONS.map((opt) => {
    const { row, arrow, letters, left, right, slot } = rowOf(opt)
    return {
      id: opt.id,
      chord: opt.arrow.run.toFixed(1),
      'head (shafts)': (HEAD.half * 2 * opt.arrow.headW).toFixed(2),
      'left vs U': edge(left),
      'right vs U': edge(right),
      'left, rastered': (opt.arrow.box.x - U_BOX.x).toFixed(2),
      'arrow width': opt.arrow.width.toFixed(2),
      'env vs today': `${(row.box.w - TODAY.box.w).toFixed(2)} x ${(row.box.h - TODAY.box.h).toFixed(2)}`,
      'vertical slot': slot.min.toFixed(2),
      'slot at 96px': inkGap(row.viewBox, arrow, letters, MIN).toFixed(2),
    }
  }),
)

// Round 11's answer, and not up for revision here: a horizontal round must leave
// the vertical slot alone. Asserted rather than reported — if it moved, that is
// a bug in this script and not a finding.
const SLOT = rowOf(OPTIONS[0]).slot.min
for (const opt of OPTIONS) {
  const drift = Math.abs(rowOf(opt).slot.min - SLOT)
  if (drift > 0.05) throw new Error(`${opt.id} changed the vertical slot by ${drift.toFixed(2)} units — it must not`)
}
console.log(`\nthe vertical slot is ${SLOT.toFixed(2)} units in every option — round 11's answer, asserted not hoped for.`)

// The envelope is the whole "does anything downstream move" question. The p's
// bowl is the rightmost ink and the mark's fire is the topmost, so nothing here
// can reach the box — but the claim is cheap to check and expensive to be wrong
// about.
for (const opt of OPTIONS) {
  const row = framed(opt, PAPER, EMBER)
  const moved = Math.max(Math.abs(row.box.w - TODAY.box.w), Math.abs(row.box.h - TODAY.box.h))
  if (moved > 0.05) throw new Error(`${opt.id} moved the lockup envelope by ${moved.toFixed(2)} units`)
}
console.log(
  `the lockup envelope is ${TODAY.box.w.toFixed(1)} x ${TODAY.box.h.toFixed(1)} units, ` +
    `${(TODAY.box.w / TODAY.box.h).toFixed(2)}:1, in every option — the header, the clear-space rule and the ` +
    'social card cannot move.\n',
)

// --- the two levers that cannot do it ----------------------------------------------
// Both of these were drawn before D and E were, and both were wrong. Kept as a
// scan rather than as a paragraph, because the reason they fail is a number.
console.log("what else could give up the 1.5, and why neither can — the arrow's own width at each setting:")
const scan = []
for (const pct of [0.33, 0.38, 0.43, 0.5, 0.6]) {
  const a = shaped({ run: CHORD, rise: CHORD * pct, place: snapped(FLUSH_LEFT) })
  scan.push({
    lever: `rise ${(pct * 100).toFixed(0)}% of the chord`,
    'arrow width': a.width.toFixed(2),
    'right vs U': edge(a.box.x + a.box.w - U_RIGHT),
    reaches: a.box.x + a.box.w - U_RIGHT <= 0.05 ? 'yes' : 'no',
  })
}
for (const long of [1.7, 1.4, 1.1]) {
  // The head's LENGTH only, its width held at 2.8 shafts.
  const a = shaped({ run: CHORD, head: { l: long / HEAD.long }, place: snapped(FLUSH_LEFT) })
  const over = a.box.x + a.box.w - U_RIGHT
  scan.push({
    lever: `head ${long.toFixed(1)} shafts long, 2.8 wide`,
    'arrow width': a.width.toFixed(2),
    'right vs U': edge(over),
    reaches: over <= 0.05 ? 'yes' : 'no',
  })
}
console.table(scan)
console.log(
  'the rise cannot do it in either direction: flattening and steepening both swing a head corner further right,\n' +
    'and 43% is already near the narrowest the shape gets. The head\'s length cannot do it either, because the\n' +
    "rightmost ink is the head's CORNER and not its tip — shortening it along its own axis moves nothing. That\n" +
    'leaves the chord (D) and the whole head scaled down (E).\n',
)

// --- the sheets ---------------------------------------------------------------------
const FONT = 'DejaVu Sans, sans-serif'
const PAD = 28
const ROW_GAP = 34
const NOTE_COL = 440

const text = (x, y, body, { size = 15, fill = PAPER, weight = 'bold' } = {}) =>
  `<text x="${x}" y="${y}" font-family="${FONT}" font-size="${size}" font-weight="${weight}" fill="${fill}">${body}</text>`

// How wide a line of text actually comes out, measured by rendering it and
// reading the alpha back — the same renderer that draws it on the sheet.
//
// Round 13's first render had its caption run off the right edge: a sheet is as
// wide as the sum of its tiles, and a caption is as long as it needs to be. The
// caption is the part of a sheet that says what is being looked at, so the
// sheet is sized to fit the sentence rather than the sentence cut to fit the
// sheet — and measured rather than estimated from a character count, which is
// what put it off the edge.
const PROBE = 4000
const textWidth = (body, size) =>
  contentBox(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${PROBE} ${size * 3}">${text(0, size * 2, body, { size })}</svg>`,
    PROBE,
  ).w

const sheetOf = ({ options, fg, accent, bg, muted, caption }) => {
  const rows = options.map((opt) => framed(opt, fg, accent))
  const width = Math.ceil(
    Math.max(PAD * 2 + REAL_WIDTHS.reduce((sum, w) => sum + w + 28, 0) + NOTE_COL, PAD * 2 + textWidth(caption, 14)),
  )
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

// Cropped to the U, with a dashed rule dropped down BOTH of its ink edges. Round
// 12's version of this sheet guided the left edge only, which is half of the
// question and the half that was already agreed.
const CROP = { x: SHIFT + U_LEFT - 18, y: -36, w: 78, h: 98 }
const TILE = 300

const cropSheet = ({ options, fg, accent, bg, muted, caption }) => {
  const tileH = (TILE * CROP.h) / CROP.w
  const hair = (1.2 * CROP.w) / TILE
  const rule = (x) =>
    `<line x1="${x.toFixed(2)}" y1="${CROP.y}" x2="${x.toFixed(2)}" y2="${CROP.y + CROP.h}" ` +
    `stroke="${accent}" stroke-width="${hair.toFixed(3)}" stroke-dasharray="${(hair * 3).toFixed(2)} ${(hair * 3).toFixed(2)}" opacity="0.85"/>`
  const guides = rule(SHIFT + U_LEFT) + rule(SHIFT + U_RIGHT)
  let x = PAD
  const tiles = options
    .map((opt) => {
      const { left, right } = rowOf(opt)
      const body = [uLayerOf(fg, accent), arrowLayerOf(opt, accent, fg)].join('')
      const out =
        `<svg viewBox="${CROP.x.toFixed(2)} ${CROP.y} ${CROP.w} ${CROP.h}" x="${x}" y="${PAD + 34}" width="${TILE}" height="${tileH.toFixed(2)}">${body}${guides}</svg>` +
        text(x, PAD + 22, opt.short, { size: 14, fill: fg }) +
        text(x, PAD + 48 + tileH, `left ${edge(left)}   ·   right ${edge(right)}`, {
          size: 13,
          fill: muted,
          weight: 'normal',
        })
      x += TILE + 20
      return out
    })
    .join('')
  const width = Math.ceil(Math.max(PAD * 2 + options.length * (TILE + 20), PAD * 2 + textWidth(caption, 14)))
  const height = PAD * 2 + 60 + tileH + 30
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
  <rect width="${width}" height="${height}" fill="${bg}"/>
  ${tiles}
  ${text(PAD, height - 14, caption, { size: 14, fill: muted, weight: 'normal' })}
</svg>`
}

// Render at the width in question, read the renderer's own pixels back, and
// redraw them as one square per pixel. Every square is a real pixel at its real
// colour, so a sub-pixel claim can be looked at instead of argued about.
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
  // The row labels are full sentences and the magnified strip is only as wide as
  // the pixels in it, so on the 96px sheet the text is what sets the width.
  const width = Math.ceil(
    Math.max(
      PAD * 2 + px * scale + 40,
      PAD * 2 + textWidth(caption, 13),
      ...options.map((opt) => PAD * 2 + textWidth(opt.label, 15)),
    ),
  )
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${y + 24}" width="${width}" height="${y + 24}">
  <rect width="${width}" height="${y + 24}" fill="${bg}"/>
  ${blocks}
  ${text(PAD, y + 8, caption, { size: 13, fill: muted, weight: 'normal' })}
</svg>`
}

// --- write ---------------------------------------------------------------------------
const out = join(root, 'flush')
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
  'both-dark',
  sheetOf({
    options: OPTIONS,
    ...DARK,
    caption: `Round 13a — both ends of the accent against both ends of the U   ·   on ink, ember ${EMBER}   ·   A and C are round 7's drawing placed two ways; D and E are the two redraws that can be flush at both ends`,
  }),
)
emit(
  'both-light',
  sheetOf({
    options: OPTIONS,
    ...LIGHT,
    caption: `Round 13a — both ends of the accent against both ends of the U   ·   on paper, ember-deep ${EMBER_DEEP}   ·   A and C are round 7's drawing placed two ways; D and E are the two redraws that can be flush at both ends`,
  }),
)

emit(
  'detail-dark',
  cropSheet({
    options: OPTIONS,
    ...DARK,
    caption: `Round 13a, cropped to the U   ·   on ink   ·   the dashed rules are the U's own two ink edges, x=${U_LEFT.toFixed(0)} and x=${U_RIGHT.toFixed(0)}`,
  }),
)
emit(
  'detail-light',
  cropSheet({
    options: OPTIONS,
    ...LIGHT,
    caption: `Round 13a, cropped to the U   ·   on paper   ·   the dashed rules are the U's own two ink edges, x=${U_LEFT.toFixed(0)} and x=${U_RIGHT.toFixed(0)}`,
  }),
)

emit(
  'zoom-320',
  zoomSheet({
    options: OPTIONS,
    ...DARK,
    width: 320,
    scale: 3,
    caption: 'Round 13a at 320px wide — the largest real use — magnified 3x, real pixels, no resampling',
  }),
)
emit(
  'zoom-96',
  zoomSheet({
    options: OPTIONS,
    ...DARK,
    width: MIN,
    scale: 5,
    caption: `Round 13a at ${MIN}px wide — the stated lockup minimum — magnified 5x, real pixels, no resampling. 1.5 units is 0.9 of a device pixel here; the decision is made at 320.`,
  }),
)

for (const opt of OPTIONS) {
  const row = framed(opt, PAPER, EMBER)
  for (const w of [MIN, 960]) {
    const file = join(out, `${opt.id}-${w}.png`)
    writeFileSync(file, renderPng(row.svg(INK), w))
    console.log(file)
  }
}

// --- what goes into the master ----------------------------------------------------------
// Printed so the master is transcribed rather than worked out twice, and then
// checked, so the two cannot drift apart afterwards. Round 12 shipped a script
// whose idea of the drawing was a unit behind the file; this is the check that
// would have caught it.
console.log('\n--- what goes into logo/wordmark.svg ---')
console.log(`${SHIP.id}: chord ${SHIP.arrow.run}, rise ${SHIP.arrow.rise.toFixed(2)} (${(RISE * 100).toFixed(0)}% of it), head 2.8 shafts, both ends flush with the U:`)
console.log(SHIP.arrow.markup)

const inMaster = GLYPHS.get('arrow').replace(/\s+/g, ' ').trim()
const wanted = SHIP.arrow.markup.replace(/\s+/g, ' ').trim()
if (inMaster === wanted) {
  console.log('\nthe master draws this exactly — checked, not assumed.')
} else if (DRAFT) {
  console.log('\n--draft: the master does not draw this yet.\n  master: ' + inMaster)
} else {
  throw new Error(
    `logo/wordmark.svg does not draw the shipped option.\n  master: ${inMaster}\n  wanted: ${wanted}\n` +
      'Transcribe it, or run with --draft while drawing.',
  )
}

// --- and what came out the other end ---------------------------------------------------
// The check above closes the gap between this script and the master. This one
// closes the gap between the master and the asset a browser actually loads,
// which is a separate step — build.mjs lifts the glyphs out, shifts them and
// composes them next to the mark, and nothing until now has read the result
// back.
//
// Measured, not parsed: the built file's own paths are isolated and rendered,
// and the edges come off the alpha. Every round since 7 has claimed a number
// about the shipped lockup in a comment; this is the first that fails if the
// claim stops being true.
const built = join(root, '..', 'site', 'assets', 'fireup-lockup-dark.svg')
if (!existsSync(built)) {
  console.log('\nsite/assets/fireup-lockup-dark.svg is not built yet — run `npm run build` to check the shipped asset.')
} else {
  const svg = readFileSync(built, 'utf8')
  const viewBox = svg.match(/viewBox="([^"]+)"/)[1]
  // The word's group: its paint and its shift are what put the glyphs where
  // they end up, so a subset has to be rendered inside it rather than alone.
  const group = svg.match(/<g fill="none"[^>]*>([\s\S]*?)<\/g>/)
  if (!group) throw new Error('could not find the word group in the built lockup')
  const open = group[0].match(/<g [^>]*>/)[0]
  const only = (paths) =>
    contentBox(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}">${open}${paths.join('')}</g></svg>`, 4096)

  const lines = group[1].trim().split('\n').map((line) => line.trim())
  const arrowPathsBuilt = lines.filter((line) => line.includes('#FF5A2B'))
  const uPath = lines.find((line) => line.includes('V35 A15 15 0 0 0'))
  if (arrowPathsBuilt.length !== 2 || !uPath) {
    throw new Error(`expected 2 accent paths and one U in the built lockup, found ${arrowPathsBuilt.length} and ${uPath ? 1 : 0}`)
  }

  const uBuilt = only([uPath])
  const aBuilt = only(arrowPathsBuilt)
  const leftGap = aBuilt.x - uBuilt.x
  const rightGap = aBuilt.x + aBuilt.w - (uBuilt.x + uBuilt.w)
  const slot = uBuilt.y - (aBuilt.y + aBuilt.h)
  console.log(
    `\nthe shipped lockup, measured off its own raster: left ${leftGap >= 0 ? '+' : ''}${leftGap.toFixed(2)}, ` +
      `right ${rightGap >= 0 ? '+' : ''}${rightGap.toFixed(2)}, vertical slot ${slot.toFixed(2)} units.`,
  )
  // 0.05 is the sampling floor: contentBox rounds outward to whole sample
  // pixels, so a measured edge lands a shade outside the geometry.
  for (const [what, value] of [['left', leftGap], ['right', rightGap]]) {
    if (Math.abs(value) > 0.05) {
      throw new Error(`the built lockup's accent is ${value.toFixed(2)} units off the U on the ${what} — it must be flush`)
    }
  }
  if (Math.abs(slot - 1.96) > 0.05) {
    throw new Error(`the built lockup's vertical slot is ${slot.toFixed(2)}, not round 11's 1.96 — a horizontal round must not move it`)
  }
  console.log('both ends flush and the slot unmoved, in the asset the site loads — checked, not assumed.')
}
