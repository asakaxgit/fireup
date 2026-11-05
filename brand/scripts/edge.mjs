#!/usr/bin/env node
// Round twelve. The note on round 11 was four words:
//
//   "bit fix. align left start of arrow and U's start"
//
//   node scripts/edge.mjs   ->  brand/edge/*.png
//
// It is a one-unit move, and it is the first horizontal note in the identity —
// every round since 7 has argued about the vertical slot between the accent and
// the letter. So this script measures the other axis.
//
// WHAT IS ACTUALLY THERE. Both edges in question are vertical and flat: the U's
// left stem is a butt-capped vertical, and the arrow's tail is a butt cap on a
// horizontal, so each presents one straight edge to the other. Measured off the
// raster, the U's leftmost ink is x=115.98 and the arrow's is x=114.98 — the
// accent overhangs the letter it belongs to by one unit on the left. A whole
// unit, not a rounding artefact.
//
// WHY IT WAS LIKE THAT, which matters because it is not a mistake being
// corrected. Round 7 placed the arrow by centring its *measured box* on the U's
// measured box. The arrow is 39.5 units wide against the U's 38 — the head
// overhangs the chord on the right — so centring the object leaves 1 unit
// hanging off the left and 0.5 off the right. That is a defensible choice and it
// is what has been on the PR since round 7. The note asks for the other one:
// align the edges and let the whole 1.5 of extra width hang off the right, the
// way the arrow already points.
//
// So this is a change of alignment rule, not a correction of an error, and the
// sheets below are what each rule looks like rather than a before and after.
//
// The mark is frozen — signed off in round 5, untouched since. Nothing here
// opens mark.svg. The arrow is not redrawn either: every option is the approved
// round 7 glyph at a whole-unit horizontal offset, so its shape, weight, chord
// and head are not in question and cannot drift.
import { renderPng, contentBox, pixelMap } from './render.mjs'
import { inkGap, slotRun } from './measure.mjs'
import { master, innerOf, recolour, spell, GLYPHS, W } from './word.mjs'
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

const MIN = 96 // the stated lockup minimum
const REAL_WIDTHS = [320, 200, 128, MIN]

const PAINT = `fill="none" stroke="#FFFFFF" stroke-width="${W.stroke}" stroke-linecap="butt" stroke-linejoin="round"`

// Painted on its own, big, for measuring one glyph's ink.
const alone = (markup) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="-60 -90 340 200"><g ${PAINT}>${markup}</g></svg>`
const inkOf = (markup) => contentBox(alone(markup), 2048)

// --- the two edges ------------------------------------------------------------
// Measured, not counted. Both are butt caps on whole coordinates, so the
// construction is exact too and the two are quoted side by side below:
// `contentBox` rounds outward to whole sample pixels, which puts a measured
// edge about 0.02 units outside the geometry at this sample size.
const U_BOX = inkOf(GLYPHS.get('U'))
const ARROW_BOX = inkOf(GLYPHS.get('arrow'))

const GEO = {
  uLeft: 120 - W.stroke / 2, // the left stem, butt-capped, centred on x=120
  uRight: 150 + W.stroke / 2,
  uStem: 120, // the stem's centre line — the other reading of "the U's start"
  arrowLeft: 115, // the tail's butt cap, on a horizontal, so its ink stops here
}

// Every option is the same glyph at a whole-unit offset. Named by the rule it
// follows rather than by the number, so a row keeps its meaning if the drawing
// under it ever moves again.
const FLUSH = GEO.uLeft - GEO.arrowLeft // +1: the two left edges on one line
const ON_STEM = GEO.uStem - GEO.arrowLeft // +5: the tail's cap on the stem's centre
const CENTRED = 0 // round 7's rule: the arrow's box centred on the U's

// --- the lockup ---------------------------------------------------------------
// Composed the way build.mjs composes it, so a row on these sheets is the asset
// and not an approximation of it. The mark's scale is the one that was signed
// off; nothing in this round touches it.
const M = { foot: 60, armEnd: 28, stemL: 6 }
const SCALE = 1.5 // round 6, signed off

const markX = (u) => (u - M.stemL) * SCALE
const SHIFT = markX(M.armEnd) + W.letterGap - W.ireupL

const markLayer = (fg, accent) =>
  `<g transform="translate(${(-M.stemL * SCALE).toFixed(2)} ${(W.baseline - M.foot * SCALE).toFixed(2)}) scale(${SCALE.toFixed(4)})">${innerOf(recolour(master('mark'), fg, accent))}</g>`

const variant = ({ id, label, short, note, dx }) => {
  const letters = (fg, accent) =>
    [markLayer(fg, accent), spell(['i', 'r', 'e', 'U', 'p', 'tittle'], { shift: SHIFT, fg, accent })].join('\n  ')
  // The arrow paints itself — both its paths name the ember explicitly — but the
  // group it sits in is painted anyway, so a future glyph that inherits cannot
  // come out the wrong colour on the light sheet.
  const arrowLayer = (accent, fg = PAPER) => spell(['arrow'], { shift: SHIFT + dx, fg, accent })
  return {
    id,
    label,
    short: short ?? label,
    note,
    dx,
    // Where each end of the accent lands relative to the matching end of the
    // letter, by construction, measuring rightwards in both cases: negative is
    // outside the letter on the left, positive is outside it on the right.
    offL: GEO.arrowLeft + dx - GEO.uLeft,
    offR: GEO.arrowLeft + dx + ARROW_BOX.w - GEO.uRight,
    letters,
    arrowLayer,
    uLayer: (fg, accent) => spell(['U'], { shift: SHIFT, fg, accent }),
    body: (fg, accent) => [letters(fg, accent), arrowLayer(accent, fg)].join('\n  '),
  }
}

const OPTIONS = [
  variant({
    id: 'a-flush',
    label: "A · The two left edges flush — the note, literally. Ships.",
    short: 'A · Edges flush',
    dx: FLUSH,
    note:
      "The accent starts where the letter starts, and all 1.5 units of\n" +
      "the arrow's extra width hang off the right — the way it points.\n" +
      'One unit of movement; the drawing is untouched.',
  }),
  variant({
    id: 'b-stem',
    label: "B · The tail's cap on the left stem's centre line",
    short: 'B · On the stem',
    dx: ON_STEM,
    note:
      'The other reading of "the U\'s start": the stem\'s own path\n' +
      'begins at x=120. The tail then stops over the middle of the\n' +
      'stem, so no two edges line up and the accent leans right.',
  }),
  variant({
    id: 'c-centred',
    label: "C · Round 7's rule — the arrow's box centred on the U's. On the PR today.",
    short: 'C · Centred (today)',
    dx: CENTRED,
    note:
      'Balanced as an object: 1 unit over the left edge, 0.5 over the\n' +
      'right. Not wrong — but two flat vertical edges a single unit\n' +
      'apart read as a miss rather than as an overhang.',
  }),
]

const SHIP = OPTIONS[0]

// --- framing ------------------------------------------------------------------
// The lockup's own envelope, measured off the rendered alpha, the same way
// build.mjs measures it. The arrow sits well inside the word horizontally — the
// p's bowl is the rightmost ink — so this is also the check that moving it
// cannot move the box, which is what "nothing downstream moves" means.
const PADDING = 8
const framed = (opt, fg, accent) => {
  const body = opt.body(fg, accent)
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

const TODAY = framed(OPTIONS[2], PAPER, EMBER)

// --- measurement --------------------------------------------------------------
console.log(
  `the U's ink: x ${U_BOX.x.toFixed(2)} to ${(U_BOX.x + U_BOX.w).toFixed(2)} measured, ` +
    `${GEO.uLeft} to ${GEO.uRight} by construction (width ${U_BOX.w.toFixed(2)})`,
)
console.log(
  `the arrow's ink: x ${ARROW_BOX.x.toFixed(2)} to ${(ARROW_BOX.x + ARROW_BOX.w).toFixed(2)} measured, ` +
    `tail cap at ${GEO.arrowLeft} by construction (width ${ARROW_BOX.w.toFixed(2)}, ` +
    `${(ARROW_BOX.w - U_BOX.w).toFixed(2)} wider than the letter because the head overhangs the chord)`,
)
console.log(
  `so the accent hangs ${(U_BOX.x - ARROW_BOX.x).toFixed(2)} units off the letter's left edge and ` +
    `${(ARROW_BOX.x + ARROW_BOX.w - (U_BOX.x + U_BOX.w)).toFixed(2)} off its right. ` +
    `Flush costs ${FLUSH} unit of horizontal movement.\n`,
)

// The horizontal offset in device pixels at every width the lockup is really
// drawn at, which is the only place "one unit" means anything. Alongside it: the
// vertical slot, which this round must not change — it was signed off at 2 in
// round 11, and a horizontal move must leave it alone.
const report = (options) =>
  console.table(
    options.map((opt) => {
      const row = framed(opt, PAPER, EMBER)
      const arrow = opt.arrowLayer(EMBER)
      const letters = opt.letters(PAPER, EMBER)
      const slot = slotRun({ viewBox: row.viewBox, boxWidth: row.box.w, upper: arrow, lower: letters, limit: W.stroke })
      const px = (w) => ((opt.offL * w) / row.box.w).toFixed(2)
      return {
        id: opt.id,
        'dx (units)': opt.dx,
        'left edge vs U': opt.offL === 0 ? 'flush' : `${opt.offL > 0 ? '+' : ''}${opt.offL.toFixed(1)}u`,
        'right edge vs U': `+${opt.offR.toFixed(1)}u`,
        aspect: `${(row.box.w / row.box.h).toFixed(2)}:1`,
        'env vs today': `${(row.box.w - TODAY.box.w).toFixed(2)} x ${(row.box.h - TODAY.box.h).toFixed(2)}`,
        'off by, at 320px': `${px(320)}px`,
        'at 200px': `${px(200)}px`,
        'at 96px': `${px(MIN)}px`,
        'vertical slot': slot.min.toFixed(2),
        'slot at 96px': inkGap(row.viewBox, arrow, letters, MIN).toFixed(2),
      }
    }),
  )

console.log('12a — where the accent starts:')
report(OPTIONS)

// The vertical slot is round 11's answer and is not up for revision here, so it
// is asserted rather than reported: if a horizontal move changed it, that would
// be a bug in this script and not a finding.
const slotOf = (opt) => {
  const row = framed(opt, PAPER, EMBER)
  return slotRun({
    viewBox: row.viewBox,
    boxWidth: row.box.w,
    upper: opt.arrowLayer(EMBER),
    lower: opt.letters(PAPER, EMBER),
    limit: W.stroke,
  }).min
}
const SLOT_TODAY = slotOf(OPTIONS[2])
for (const opt of OPTIONS) {
  const drift = Math.abs(slotOf(opt) - SLOT_TODAY)
  if (drift > 0.05) throw new Error(`${opt.id} changed the vertical slot by ${drift.toFixed(2)} units — it must not`)
}
console.log(
  `\nthe vertical slot is ${SLOT_TODAY.toFixed(2)} units in every option — round 11's answer, untouched by a ` +
    'horizontal move, asserted above rather than hoped for.',
)

// The envelope, which is the whole "does anything downstream move" question. The
// rightmost ink is the p's bowl at x=200 and the arrow's head lands at 155.5, so
// there is no way for this to grow the box — but the claim is cheap to check and
// expensive to be wrong about, so it is checked.
for (const opt of OPTIONS) {
  const row = framed(opt, PAPER, EMBER)
  const moved = Math.max(Math.abs(row.box.w - TODAY.box.w), Math.abs(row.box.h - TODAY.box.h))
  if (moved > 0.05) throw new Error(`${opt.id} moved the lockup envelope by ${moved.toFixed(2)} units`)
}
console.log(
  `the lockup envelope is ${TODAY.box.w.toFixed(1)} x ${TODAY.box.h.toFixed(1)} units, ` +
    `${(TODAY.box.w / TODAY.box.h).toFixed(2)}:1, in every option — so the header, the clear-space rule and the ` +
    'social card cannot move.\n',
)

// --- the sheets ---------------------------------------------------------------
const FONT = 'DejaVu Sans, sans-serif'
const PAD = 28
const ROW_GAP = 34
const NOTE_COL = 440

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

// Cropped to the U and its accent, with a dashed rule dropped down the U's left
// ink edge. A one-unit offset is about 2 device pixels at 320px and nothing at
// all at 96; it is only arguable against a guide, so the guide is drawn.
const CROP = { x: SHIFT + GEO.uLeft - 20, y: -36, w: 76, h: 98 }
const TILE = 300

const cropSheet = ({ options, fg, accent, bg, muted, caption }) => {
  const tileH = (TILE * CROP.h) / CROP.w
  const hair = (1.2 * CROP.w) / TILE
  const guide =
    `<line x1="${(SHIFT + GEO.uLeft).toFixed(2)}" y1="${CROP.y}" x2="${(SHIFT + GEO.uLeft).toFixed(2)}" y2="${CROP.y + CROP.h}" ` +
    `stroke="${accent}" stroke-width="${hair.toFixed(3)}" stroke-dasharray="${(hair * 3).toFixed(2)} ${(hair * 3).toFixed(2)}" opacity="0.85"/>`
  let x = PAD
  const tiles = options
    .map((opt) => {
      const body = [opt.uLayer(fg, accent), opt.arrowLayer(accent, fg)].join('')
      const out =
        `<svg viewBox="${CROP.x.toFixed(2)} ${CROP.y} ${CROP.w} ${CROP.h}" x="${x}" y="${PAD + 34}" width="${TILE}" height="${tileH.toFixed(2)}">${body}${guide}</svg>` +
        text(x, PAD + 22, opt.short, { size: 14, fill: fg }) +
        text(
          x,
          PAD + 48 + tileH,
          opt.offL === 0
            ? 'flush with the rule'
            : `${Math.abs(opt.offL)} unit${Math.abs(opt.offL) === 1 ? '' : 's'} ${opt.offL < 0 ? 'left of' : 'right of'} the rule`,
          { size: 13, fill: muted, weight: 'normal' },
        )
      x += TILE + 20
      return out
    })
    .join('')
  const width = PAD * 2 + options.length * (TILE + 20)
  const height = PAD * 2 + 60 + tileH + 30
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
  <rect width="${width}" height="${height}" fill="${bg}"/>
  ${tiles}
  ${text(PAD, height - 14, caption, { size: 14, fill: muted, weight: 'normal' })}
</svg>`
}

// A sub-pixel offset cannot be argued about from a sheet, so: render at the
// width in question, read the renderer's own pixels back, and redraw them as one
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

// --- write --------------------------------------------------------------------
const out = join(root, 'edge')
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
  'start-dark',
  sheetOf({
    options: OPTIONS,
    ...DARK,
    caption: `Round 12a — where the accent starts   ·   on ink, ember ${EMBER}   ·   the only difference between the rows is a whole-unit horizontal offset on the arrow`,
  }),
)
emit(
  'start-light',
  sheetOf({
    options: OPTIONS,
    ...LIGHT,
    caption: `Round 12a — where the accent starts   ·   on paper, ember-deep ${EMBER_DEEP}   ·   the only difference between the rows is a whole-unit horizontal offset on the arrow`,
  }),
)

emit(
  'detail-dark',
  cropSheet({
    options: OPTIONS,
    ...DARK,
    caption: `Round 12a, cropped to the U and its accent   ·   on ink   ·   the dashed rule is the U's own left ink edge, x=${GEO.uLeft}`,
  }),
)
emit(
  'detail-light',
  cropSheet({
    options: OPTIONS,
    ...LIGHT,
    caption: `Round 12a, cropped to the U and its accent   ·   on paper   ·   the dashed rule is the U's own left ink edge, x=${GEO.uLeft}`,
  }),
)

emit(
  'zoom-320',
  zoomSheet({
    options: OPTIONS,
    ...DARK,
    width: 320,
    scale: 3,
    caption: 'Round 12a at 320px wide — the largest real use — magnified 3x, real pixels, no resampling',
  }),
)
emit(
  'zoom-96',
  zoomSheet({
    options: OPTIONS,
    ...DARK,
    width: MIN,
    scale: 5,
    caption: `Round 12a at ${MIN}px wide — the stated lockup minimum — magnified 5x, real pixels, no resampling. One unit is 0.6 device pixels here; the decision is made at 320.`,
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

// --- what goes into the master ------------------------------------------------
// Printed so the master is transcribed rather than worked out twice. The arrow
// is the approved glyph moved horizontally, so what matters is one offset and
// not a new path: every x in its two paths shifts by the same whole number,
// which keeps the drawing bit-identical in shape.
console.log(`\n--- what goes into logo/wordmark.svg ---`)
console.log(
  `arrow: every x + ${SHIP.dx}, exactly — the approved glyph moved by a whole unit, so its shape, weight, chord and ` +
    `head are untouched. Left edge flush with the U's at x=${GEO.uLeft}:`,
)
console.log(
  GLYPHS.get('arrow')
    .split('\n')
    .map((line) =>
      line
        .replace(/([ML])([-\d.]+) ([-\d.]+)/g, (_, cmd, x, y) => `${cmd}${Number(x) + SHIP.dx} ${y}`)
        .replace(/([QL]) ?([-\d.]+) ([-\d.]+) ([-\d.]+) ([-\d.]+)/g, (_, cmd, x1, y1, x2, y2) => `${cmd}${Number(x1) + SHIP.dx} ${y1} ${Number(x2) + SHIP.dx} ${y2}`),
    )
    .join('\n'),
)
