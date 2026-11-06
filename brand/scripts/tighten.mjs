#!/usr/bin/env node
// Round ten. The note on round 9: "Tighten the gap of arrow and U."
//
//   node scripts/tighten.mjs   ->  brand/tighten/*.png
//
// One number — and the first thing this script did was prove that number has
// been written down wrong since round 7.
//
// THE GAP IS 7 UNITS, NOT 3. Every comment in this identity says the arrow
// clears the U by 3, "the gap the i's tittle keeps over its own stem", read off
// the master as ACCENT_CLEARANCE. That constant is computed from the
// construction lines: `W.xTop - W.stroke / 2 - (DOT.cy + DOT.r)`, i.e. it
// assumes the stem's ink begins half a stroke above the x-height line. It does
// not. The i's stem is a butt-capped vertical, so it *terminates* on y=20 and
// its topmost ink is at y=20 — not 16. The real air under the tittle is 7.
//
// The capital U has the same shape: butt-capped stems that stop on the
// ascender line, so its topmost ink is at y=4, not at y=0 where
// `ascTop - stroke / 2` puts it. Rounds 7 and 9 placed the arrow 3 units above
// y=0, which is 7 units above the letter.
//
// Measured, not reasoned: the numbers above come out of `contentBox` on each
// glyph rendered alone, and this script re-measures them every run rather than
// carrying them as constants. So the request is not a taste adjustment. The
// drawing has 7 units of air where it claims to have 3, which is a fifth of the
// U's height, and that is what is being seen as loose.
//
// Both halves of the rule were wrong by the same 4, which is worth stating
// plainly: the arrow and the tittle really do keep the same air as each other.
// What was broken is the stated number, not the consistency.
//
// So the options below are indexed by air that is measured off the raster —
// arrow's lowest ink to the U's highest ink — and 10b asks which of the two
// mechanisms should produce it, because moving the letter up and moving the
// arrow down tighten the same gap and are not the same decision.
//
// What decides it is not the sheet at 320. It is whether the slot survives at
// the sizes the lockup is really used at. At the stated 96px minimum one
// wordmark unit is 0.45px, so today's 7 units is about 3px of background, 4
// units is 1.8px and 2 units is under a pixel — which the renderer cannot draw
// as anything but a smudge. That is measured below (`inkGap`, in device pixels
// at each real width) rather than judged by eye, because a sub-pixel slot is
// exactly the thing eyes fill in.
//
// The mark is frozen — signed off in round 5, moved into the word in round 6,
// untouched since. Nothing here opens mark.svg.
//
// Wordmark construction (logo/wordmark.svg, via word.mjs): baseline y=50,
// x-height top y=20, ascender top y=4, descender y=66, stroke 8. The U's stems
// stand on x=120 and x=150, so with the stroke it spans x=116..154; a capital's
// top edge is the ascender line less half a stroke, y=0.
// Mark construction (logo/mark.svg): F foot y=60, cap top y=16, bar weight 12,
// middle arm ends x=28, bar ends x=36, fire tip y=1, fire out to x=56.
import { renderPng, contentBox, pixelMap } from './render.mjs'
import { inkGap, slotRun } from './measure.mjs'
import { master, innerOf, recolour, spell as spellNow, W, dotAsOf, accentClearanceAsOf } from './word.mjs'
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

// This script is the record of round 10, so it draws round 10's letters. Round
// 11 moved the i's tittle and the U; unpinned, re-running this would re-render
// every sheet below with letters round 10 never saw, and the captions here —
// which measure the tittle at 7 — would stop being true of what the sheet
// shows. See HISTORY in word.mjs.
const AS_OF = 'round-10'
const spell = (names, options = {}) => spellNow(names, { ...options, asOf: AS_OF })
const DOT = dotAsOf(AS_OF)
// Rounds 7 and 8 placed the arrow with this, so it is geometry here, not a
// caption: left reading the live tittle it would move these drawings.
const ACCENT_CLEARANCE = accentClearanceAsOf(AS_OF)

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')
const tokens = JSON.parse(readFileSync(join(root, 'tokens', 'brand.json'), 'utf8'))

const INK = tokens.color.ink
const PAPER = tokens.color.paper
const EMBER = tokens.color.ember
const EMBER_DEEP = tokens.color['ember-deep']
const MUTED = tokens.color['paper-muted']

// --- the lockup, exactly as rounds 6 to 9 settled it --------------------------
const M = { foot: 60, capTop: 16, bar: 12, armEnd: 28, barEnd: 36, fireTip: 1, fireEnd: 56, stemL: 6 }
const MARK_SCALE = 1.5

const markX = (u) => (u - M.stemL) * MARK_SCALE
const WORD_X = markX(M.armEnd) + W.letterGap
const SHIFT = WORD_X - W.ireupL

const MARK_PLACED = (fg, accent) =>
  `<g transform="translate(${(-M.stemL * MARK_SCALE).toFixed(2)} ${(W.baseline - M.foot * MARK_SCALE).toFixed(2)}) scale(${MARK_SCALE.toFixed(4)})">${innerOf(recolour(master('mark'), fg, accent))}</g>`

const at = (x) => x + SHIFT
const U = { left: at(120 - W.stroke / 2), right: at(150 + W.stroke / 2) }

// The capital U, drawn here because these sheets draw it at several clearances
// and at two heights, and the master carries only the one that ships.
//
// `top` is where its stems terminate. The shipped letter runs them to the
// ascender line, 4; 10b also draws them at 0, which is where the F's cap bar
// actually puts its ink, so the two capitals' tops align.
const capU = (top = W.ascTop) => `<path d="M120 ${top} V35 A15 15 0 0 0 150 35 V${top}"/>`
const CAP_U = capU()

// Where the ink really is, measured off each glyph rendered on its own rather
// than derived from the construction lines. This is the correction the round is
// built on, so it is re-measured every run and printed.
const PAINTED = (markup) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="-40 -60 300 180">` +
  `<g fill="none" stroke="#FFFFFF" stroke-width="${W.stroke}" stroke-linecap="butt" stroke-linejoin="round">${markup}</g></svg>`

const inkTopOf = (markup) => contentBox(PAINTED(markup), 2048).y
const U_TOP = inkTopOf(CAP_U) // 4.0 — butt-capped stems stop on the line
const U_TOP_RAISED = inkTopOf(capU(0))
const STEM_TOP = contentBox(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="-40 -60 300 180">${spell(['i'])}</svg>`, 2048).y
const DOT_BOTTOM = DOT.cy + DOT.r
const TITTLE_AIR = STEM_TOP - DOT_BOTTOM // the real air under the word's own accent

// --- the arrow, unchanged in every way except how high it sits ----------------
// Shaft 8, the word's own monoline weight. Head 2.8 shafts wide and 1.7 long.
// The curve bows like a growth curve: control point level with the tail, 72%
// along. Chord 30 x 12.9, stem to stem across the U. All of that is round 7 and
// none of it is in question this round — the only variable below is `bottom`.
const ARROW_W = W.stroke
const HEAD = { half: ARROW_W * 1.4, long: ARROW_W * 1.7 }
const BOW = 0.72
const RUN = 150 - 120
const RISE = RUN * 0.43

const arrowBody = () => {
  const tail = { x: 0, y: 0 }
  const ctrl = { x: RUN * BOW, y: 0 }
  const neck = { x: RUN, y: -RISE }
  const span = { x: neck.x - ctrl.x, y: neck.y - ctrl.y }
  const length = Math.hypot(span.x, span.y)
  const u = { x: span.x / length, y: span.y / length }
  const n = { x: -u.y, y: u.x }
  const point = (p) => `${p.x.toFixed(2)} ${p.y.toFixed(2)}`
  const tip = { x: neck.x + u.x * HEAD.long, y: neck.y + u.y * HEAD.long }
  const left = { x: neck.x + n.x * HEAD.half, y: neck.y + n.y * HEAD.half }
  const right = { x: neck.x - n.x * HEAD.half, y: neck.y - n.y * HEAD.half }
  return (
    `<path d="M${point(tail)} Q${point(ctrl)} ${point(neck)}" fill="none" stroke="#FF5A2B" ` +
    `stroke-width="${ARROW_W}" stroke-linecap="butt"/>` +
    `<path d="M${point(left)} L${point(tip)} L${point(right)} Z" fill="#FF5A2B" stroke="none"/>`
  )
}

const BODY = arrowBody()
const BOX = contentBox(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="-40 -80 200 140">${BODY}</svg>`)

// The same arrow, centred on the U, with `air` units of background between its
// lowest ink and the letter's highest ink. Both ends of that measurement come
// off the raster rather than off a construction line, which is the whole point
// of this round: the arrow's lowest ink is the tail's bottom edge, and `below`
// is the measured top of whichever U it sits over.
const arrowAt = (air, { shifted = true, below = U_TOP } = {}) => {
  // `shifted` picks the coordinate space: the lockup shifts the word right by
  // SHIFT, so a sheet wants the shifted centre and the master wants its own.
  // The height is the same in both — the shift is horizontal.
  const over = shifted ? (U.left + U.right) / 2 : (120 + 150) / 2
  const bottom = below - air
  const tx = over - (BOX.x + BOX.w / 2)
  const ty = bottom - (BOX.y + BOX.h)
  return {
    air,
    below,
    width: BOX.w,
    height: BOX.h,
    bottom,
    top: ty + BOX.y,
    left: tx + BOX.x,
    right: tx + BOX.x + BOX.w,
    draw: (accent) => `<g transform="translate(${tx.toFixed(2)} ${ty.toFixed(2)})">${BODY.replaceAll('#FF5A2B', accent)}</g>`,
  }
}

// How much background is actually left between the two shapes — `inkGap` and
// `slotRun`, in scripts/measure.mjs. Round 11 wanted both unchanged, so they
// moved out of this file rather than being copied into that one.

// --- compositions -------------------------------------------------------------
const PAINT = 'fill="none" stroke="#FFFFFF" stroke-width="8" stroke-linecap="butt" stroke-linejoin="round"'
const drawn = (markup, fg, accent, shift = 0) =>
  `<g ${recolour(PAINT, fg, accent)} transform="translate(${shift.toFixed(2)} 0)">${markup}</g>`

const letters = (fg, accent, { uTop = W.ascTop } = {}) =>
  [
    MARK_PLACED(fg, accent),
    spell(['i', 'r', 'e', 'p', 'tittle'], { shift: SHIFT, fg, accent }),
    drawn(capU(uTop), fg, accent, SHIFT),
  ].join('\n  ')

const compose = (arrow, uTop) => (fg, accent) => [letters(fg, accent, { uTop }), arrow.draw(accent)].join('\n  ')

// --- 10a: how much air over the letter ----------------------------------------
// One variable, six values, in measured units. Today's 7 is last, as the thing
// being compared against rather than the thing being proposed. The letter does
// not move in any of them; only the arrow does.
const option = (air, id, label, note) => {
  const arrow = arrowAt(air)
  return { id, label, note, arrow, air, uTop: W.ascTop, body: compose(arrow) }
}

const CLEARANCES = [
  option(
    5,
    'air-5',
    'A · 5 units — a third out',
    'The smallest move that is visible at all at\n' +
      '200px: 0.9px of background taken out of a\n' +
      '3.2px slot. Here to set the bottom of the\n' +
      'range rather than as a candidate.',
  ),
  option(
    4,
    'air-4',
    'B · 4 units — half a stroke',
    'Half the monoline weight, which is the only\n' +
      'round number the drawing already uses for a\n' +
      'gap this size. Takes out 3 of the 7 and\n' +
      'still leaves 1.8px at the 96px minimum.',
  ),
  option(
    3,
    'air-3',
    'C · 3 units — the number the drawing claims. Ships.',
    'What every comment since round 7 has said\n' +
      'the gap already is. Tightens by more than\n' +
      'half and is still a visible slot at 200;\n' +
      'at 96px it is 1.3px and the renderer is\n' +
      'deciding, not the drawing.',
  ),
  option(
    2,
    'air-2',
    'D · 2 units — a quarter of a stroke',
    'Tight enough that the arrow starts to read\n' +
      'as attached at anything under 200px, which\n' +
      'is most of where a lockup is used.',
  ),
  option(
    0,
    'air-0',
    'E · 0 — the arrow grows out of the U',
    'Not a tighter gap: no gap. The accent stops\n' +
      'floating and becomes a stroke off the\n' +
      "letter — the mark's own rule, nothing\n" +
      'floats, applied to the word.',
  ),
  option(
    7,
    'air-7',
    'F · 7 units — today, and the tittle\'s real gap',
    'What is on the PR now. Documented as 3\n' +
      'everywhere; measures 7, which is the same\n' +
      'air the tittle really keeps over the i.',
  ),
]

// --- 10b: which end of the gap moves ------------------------------------------
// Lowering the arrow and raising the letter close the same slot, and they are
// not the same decision. The U's stems stop on the ascender line, so its ink
// tops out 4 units below the F's cap bar, which straddles that line: in the
// word-alone variant "FireUp" the two capitals do not align. Running the stems
// to y=0 instead fixes that and tightens the accent to exactly the 3 units the
// drawing has always claimed, in one change — but it alters an approved letter,
// which is not mine to do unasked.
const MECHANISMS = [
  {
    id: 'mech-arrow',
    label: 'A · Lower the arrow to 3 — no letter moves',
    air: 3,
    uTop: W.ascTop,
    arrow: arrowAt(3),
    body: compose(arrowAt(3), W.ascTop),
    note:
      'The accent comes down 4 units. Every\n' +
      'letter is untouched, so nothing else in\n' +
      'the identity can be affected by it.',
  },
  {
    id: 'mech-letter',
    label: 'B · Raise the U to the cap line — the arrow does not move',
    air: 3,
    uTop: 0,
    arrow: arrowAt(7),
    body: compose(arrowAt(7), 0),
    note:
      "The U's stems run to y=0, where the F's\n" +
      'bar already puts its ink, so the two\n' +
      'capitals align and the gap closes to 3 on\n' +
      'its own. Changes an approved letter.',
  },
  {
    id: 'mech-today',
    label: 'C · Today — 7 units, U short of the cap line',
    air: 7,
    uTop: W.ascTop,
    arrow: arrowAt(7),
    body: compose(arrowAt(7), W.ascTop),
    note: 'What is on the PR now, for comparison.',
  },
]

// --- diagnostics --------------------------------------------------------------
const framed = (opt, fg, accent) => {
  const body = opt.body(fg, accent)
  const box = contentBox(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="-60 -90 420 230">${body}</svg>`)
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

const TODAY = framed(CLEARANCES.at(-1), PAPER, EMBER)
const MIN = 96 // the stated lockup minimum, in px wide
const REAL_WIDTHS = [320, 200, 128, MIN]

// The correction this round rests on, re-measured every run rather than
// asserted in a comment.
console.log(`ACCENT_CLEARANCE as the code computes it: ${ACCENT_CLEARANCE} units`)
console.log(
  `measured off the raster: tittle bottom ink y=${DOT_BOTTOM}, i stem top ink y=${STEM_TOP.toFixed(2)} ` +
    `-> the word's accent really keeps ${TITTLE_AIR.toFixed(2)} units of air, not ${ACCENT_CLEARANCE}`,
)
console.log(
  `capital U top ink y=${U_TOP.toFixed(2)} (construction says ${W.ascTop - W.stroke / 2}); ` +
    `stems run to y=0 instead -> y=${U_TOP_RAISED.toFixed(2)}, which is where the F's cap bar puts its ink`,
)
console.log(`today's lockup envelope: ${TODAY.box.w.toFixed(1)} x ${TODAY.box.h.toFixed(1)} units, ${(TODAY.box.w / TODAY.box.h).toFixed(2)}:1, top at y=${TODAY.box.y.toFixed(1)}\n`)

// The slot between the two shapes, in device pixels, at every width the lockup
// is really drawn at: arrow layer against letter layer, through the option's
// own frame. Also the air averaged over the run of the slot rather than at its
// tightest point — a dot over a stem pinches and opens out again, a flat tail
// over a flat cap does not, and two slots with the same minimum do not read the
// same if one of them is parallel.
const slotOf = (opt) => {
  const row = framed(opt, PAPER, EMBER)
  const arrowLayer = opt.arrow.draw(EMBER)
  const letterLayer = letters(PAPER, EMBER, { uTop: opt.uTop })
  const slot = slotRun({
    viewBox: row.viewBox,
    boxWidth: row.box.w,
    upper: arrowLayer,
    lower: letterLayer,
    limit: W.stroke,
  })
  return {
    row,
    arrowLayer,
    letterLayer,
    run: slot.run,
    mean: slot.mean,
    px: (w) => inkGap(row.viewBox, arrowLayer, letterLayer, w),
  }
}

const report = (options) =>
  console.table(
    options.map((opt) => {
      const slot = slotOf(opt)
      const unit = MIN / slot.row.box.w
      return {
        id: opt.id,
        'air (units)': opt.air.toFixed(1),
        'U top': opt.uTop === W.ascTop ? 'as drawn' : 'raised to cap',
        aspect: `${(slot.row.box.w / slot.row.box.h).toFixed(2)}:1`,
        'envelope vs today': `${(slot.row.box.h - TODAY.box.h).toFixed(1)}`,
        'slot runs': `${slot.run.toFixed(1)} units`,
        'mean air': slot.mean.toFixed(1),
        '320px': slot.px(320).toFixed(2),
        '200px': slot.px(200).toFixed(2),
        '128px': slot.px(128).toFixed(2),
        '96px': slot.px(MIN).toFixed(2),
        'x-height at 96px': ((W.baseline - W.xTop) * unit).toFixed(1),
      }
    }),
  )

console.log('10a — the arrow comes down, no letter moves:')
report(CLEARANCES)
console.log('\n10b — which end of the gap moves:')
report(MECHANISMS)

// The tittle is what the clearance was borrowed from, so the same measurement
// is run on it: the air the dot keeps over the i's stem, in device pixels at
// the same widths, through the same frame.
const DOT_LAYER = spell(['tittle'], { shift: SHIFT, fg: PAPER, accent: EMBER })
const STEM_LAYER = spell(['i'], { shift: SHIFT, fg: PAPER, accent: EMBER })
console.log(
  `\nthe tittle's own gap (${TITTLE_AIR.toFixed(1)} units), measured the same way: ` +
    REAL_WIDTHS.map((w) => `${inkGap(TODAY.viewBox, DOT_LAYER, STEM_LAYER, w).toFixed(2)}px at ${w}`).join(', '),
)

// Whether the two accents' slots are the same shape, not just the same
// minimum. The arrow's tail is flat and the U's cap is flat, so its slot is
// parallel and its mean air is its minimum. The tittle is a circle over a flat
// stem, so its slot pinches at one point and opens out on both sides — the same
// measured minimum, more background around it. That is the reason the two can
// read differently at the same number, and it is measured rather than asserted.
const tittleSlot = slotRun({
  viewBox: TODAY.viewBox,
  boxWidth: TODAY.box.w,
  upper: DOT_LAYER,
  lower: STEM_LAYER,
  limit: W.stroke * 2,
})
console.log(
  `the tittle's slot: ${tittleSlot.run.toFixed(1)} units long, ${tittleSlot.min.toFixed(1)} at its tightest, ` +
    `${tittleSlot.mean.toFixed(1)} on average — it pinches and opens out. The arrow's is parallel, so its mean is its minimum.`,
)

// --- the sheets ---------------------------------------------------------------
const FONT = 'DejaVu Sans, sans-serif'
const PAD = 28
const ROW_GAP = 34
const NOTE_COL = 330

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

// A sub-pixel slot cannot be argued about by looking at a sheet, so: render each
// option at the width in question, read the renderer's own pixels back, and
// redraw them as one square per pixel. Every square is a real pixel at its real
// colour.
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

// The U and its accent on their own, big. The whole lockup at 320 devotes about
// 12px to the thing being decided; this crops to it so the five options can be
// compared as drawings rather than as details.
const detailSheet = ({ options, fg, accent, bg, muted, caption }) => {
  // The whole letter, not just the two stem tops: the gap is only readable
  // against the thing it sits on.
  const CROP = { x: U.left - 16, y: U_TOP - 36, w: 76, h: 98 }
  const TILE = 240
  const tileH = (TILE * CROP.h) / CROP.w
  let x = PAD
  const tiles = options
    .map((opt) => {
      const body = [drawn(capU(opt.uTop), fg, accent, SHIFT), opt.arrow.draw(accent)].join('')
      const tile =
        `<svg viewBox="${CROP.x.toFixed(2)} ${CROP.y.toFixed(2)} ${CROP.w} ${CROP.h}" x="${x}" y="${PAD + 34}" width="${TILE}" height="${tileH.toFixed(2)}">${body}</svg>` +
        text(x, PAD + 22, opt.label.split(' — ')[0], { size: 14, fill: fg }) +
        text(x, PAD + 48 + tileH, `${opt.air.toFixed(1)} units of air`, { size: 13, fill: muted, weight: 'normal' })
      x += TILE + 20
      return tile
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

const out = join(root, 'tighten')
mkdirSync(out, { recursive: true })

const DARK = { fg: PAPER, accent: EMBER, bg: INK, muted: MUTED }
const LIGHT = { fg: INK, accent: EMBER_DEEP, bg: PAPER, muted: '#5A6782' }

for (const config of [
  {
    file: 'clear-dark',
    options: CLEARANCES,
    ...DARK,
    caption: `Round 10a — how much air between the arrow and the U, measured   ·   on ink, ember ${EMBER}   ·   one variable; the mark and every letter are identical in all six`,
  },
  {
    file: 'clear-light',
    options: CLEARANCES,
    ...LIGHT,
    caption: `Round 10a — how much air between the arrow and the U, measured   ·   on paper, ember-deep ${EMBER_DEEP}   ·   one variable; the mark and every letter are identical in all six`,
  },
  {
    file: 'mech-dark',
    options: MECHANISMS,
    ...DARK,
    caption: 'Round 10b — which end of the gap moves   ·   on ink   ·   A and B both land on 3 units of air; B also aligns the U with the F',
  },
  {
    file: 'mech-light',
    options: MECHANISMS,
    ...LIGHT,
    caption: 'Round 10b — which end of the gap moves   ·   on paper   ·   A and B both land on 3 units of air; B also aligns the U with the F',
  },
]) {
  const source = sheetOf(config)
  const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const file = join(out, `${config.file}.png`)
  writeFileSync(file, renderPng(source, width))
  console.log(file)
}

for (const config of [
  {
    file: 'detail-dark',
    options: CLEARANCES,
    ...DARK,
    caption: 'Round 10a, cropped to the U and its accent   ·   on ink   ·   magnified ~3.3x from the 320px tile, where the decision is only 11px tall',
  },
  {
    file: 'detail-light',
    options: CLEARANCES,
    ...LIGHT,
    caption: 'Round 10a, cropped to the U and its accent   ·   on paper   ·   magnified ~3.3x from the 320px tile, where the decision is only 11px tall',
  },
  {
    file: 'detail-mech',
    options: MECHANISMS,
    ...DARK,
    caption: 'Round 10b, cropped to the U and its accent   ·   on ink   ·   same 3 units of air in A and B, reached from opposite ends',
  },
]) {
  const source = detailSheet(config)
  const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const file = join(out, `${config.file}.png`)
  writeFileSync(file, renderPng(source, width))
  console.log(file)
}

for (const [file, options, px, scale, caption] of [
  ['zoom-200', CLEARANCES, 200, 4, 'Round 10a at 200px wide — the site header size — magnified 4x, real pixels, no resampling'],
  ['zoom-96', CLEARANCES, MIN, 5, `Round 10a at ${MIN}px wide — the stated lockup minimum — magnified 5x, real pixels, no resampling`],
  ['zoom-mech', MECHANISMS, MIN, 5, `Round 10b at ${MIN}px wide — magnified 5x, real pixels, no resampling`],
]) {
  const source = zoomSheet({ options, ...DARK, caption, width: px, scale })
  const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const path = join(out, `${file}.png`)
  writeFileSync(path, renderPng(source, width))
  console.log(path)
}

// Each option on its own at the two widths that decide it: 96, the stated
// minimum, and 960, where a slot that has gone optically uneven shows up.
for (const opt of [...CLEARANCES, ...MECHANISMS]) {
  const row = framed(opt, PAPER, EMBER)
  for (const w of [MIN, 960]) {
    const file = join(out, `${opt.id}-${w}.png`)
    writeFileSync(file, renderPng(row.svg(INK), w))
    console.log(file)
  }
}

// --- what goes into the master -------------------------------------------------
// The arrow in the wordmark's own coordinates — the lockup shifts the word right
// by SHIFT and that shift is not part of the drawing. The numbers go into
// logo/wordmark.svg by hand, which is the only way the master stays a drawing
// someone can read and correct; printing them means they are transcribed rather
// than worked out twice. The master rounds to whole units where the difference
// is under half a pixel at the largest real use.
// 3 is what shipped: 10a-C, the tightest air that still holds a slot of
// background at the 96px minimum. SHIP_AIR prints the geometry for a different
// gap without touching what the sheets show.
const SHIP = Number(process.env.SHIP_AIR ?? 3)
const shipArrow = arrowAt(SHIP, { shifted: false })
console.log(
  `\nshipping ${SHIP} units of measured air — chord ${RUN}x${RISE.toFixed(2)}, shaft ${ARROW_W}, box ` +
    `${shipArrow.width.toFixed(1)}x${shipArrow.height.toFixed(1)} on x=${shipArrow.left.toFixed(1)}..${shipArrow.right.toFixed(1)}, ` +
    `y=${shipArrow.top.toFixed(1)}..${shipArrow.bottom.toFixed(1)}, in wordmark coordinates:`,
)
console.log(shipArrow.draw('#FF5A2B').replaceAll('/><', '/>\n  '))
