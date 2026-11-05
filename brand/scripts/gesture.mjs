#!/usr/bin/env node
// Round eight. The answers on the round 7 card:
//
//   lockup   -> "Use the big arc over the whole 'Up' instead (7b row D) — I
//                accept that it takes the lockup to 1.80:1 and moves the header
//                sizing again"
//   capital F -> keep it
//   prose     -> logotype only; the product stays "Fireup" in prose
//   notes     -> "7a A and 7c C is good"
//
//   node scripts/gesture.mjs   ->  brand/gesture/*.png
//
// Three of those four need no drawing. The capital F already ships, the prose
// answer is a no-change, and the mark has been frozen since round 5.
//
// The fourth is one decision with two answers that nearly agree. Both point the
// same way — out to the p, big, with air under it rather than touching:
//
//   7b row D  capital U, arc over the whole "Up", 3 clear above the cap.
//             Chord 76: the U's left edge to the p's right edge.
//   7c C      "out to the p", chord 48, centred over the U.
//   7a A      accent clearance, not touching — which is what both of the above
//             already do, so this one is satisfied either way.
//
// So the direction is settled and the chord is not: 76 or 48. The lockup
// question is the one that decides, it names row D, and it accepts row D's cost
// by name — so A is what ships unless a render says it cannot. This script
// exists to find that out rather than assert it, and to show the two side by
// side at 96px so one word can redirect it.
//
// Wordmark construction (logo/wordmark.svg, via word.mjs): baseline y=50,
// x-height top y=20, ascender top y=4, descender y=66, stroke 8. The U's stems
// stand on x=120 and x=150, so with the stroke it spans x=116..154; "Up" runs
// on to the p's bowl at x=200. A capital's top edge is the ascender line less
// half a stroke: y=0.
// Mark construction (logo/mark.svg): F foot y=60, cap top y=16, bar weight 12,
// middle arm ends x=28, bar ends x=36, fire tip y=1, fire out to x=56.
import { Resvg } from '@resvg/resvg-js'
import { renderPng, contentBox, pixelMap } from './render.mjs'
import { master, innerOf, recolour, spell, W, DOT, ACCENT_CLEARANCE } from './word.mjs'
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

// --- the lockup, exactly as build.mjs composes it -----------------------------
const M = { foot: 60, capTop: 16, bar: 12, armEnd: 28, barEnd: 36, fireTip: 1, fireEnd: 56, stemL: 6 }
const MARK_SCALE = 1.5

const markX = (u) => (u - M.stemL) * MARK_SCALE
const markY = (v) => W.baseline + (v - M.foot) * MARK_SCALE
const WORD_X = markX(M.armEnd) + W.letterGap
const SHIFT = WORD_X - W.ireupL

const MARK_PLACED = (fg, accent) =>
  `<g transform="translate(${(-M.stemL * MARK_SCALE).toFixed(2)} ${(W.baseline - M.foot * MARK_SCALE).toFixed(2)}) scale(${MARK_SCALE.toFixed(4)})">${innerOf(recolour(master('mark'), fg, accent))}</g>`

// The fire alone, placed the same way — for measuring how close the arrow gets
// to it. It is the last path in the mark, the only ember-coloured one.
const FIRE_ONLY = (() => {
  const fire = innerOf(master('mark'))
    .split('\n')
    .filter((line) => line.includes('#FF5A2B'))
    .join('\n')
  if (!fire.trim()) throw new Error('mark.svg has no ember-coloured path to measure the fire from')
  return `<g transform="translate(${(-M.stemL * MARK_SCALE).toFixed(2)} ${(W.baseline - M.foot * MARK_SCALE).toFixed(2)}) scale(${MARK_SCALE.toFixed(4)})">${fire}</g>`
})()

// Letter edges in wordmark coordinates — the drawing's own, before the lockup
// shifts the word right. The shift cancels out of every chord, so the arrow is
// designed here and placed there.
const U_LEFT = 120 - W.stroke / 2
const U_RIGHT = 150 + W.stroke / 2
const P_RIGHT = 196 + W.stroke / 2 // bowl centre 181 + radius 15, plus half a stroke
const CAP_TOP = W.ascTop - W.stroke / 2
const MID_U = (U_LEFT + U_RIGHT) / 2
const MID_UP = (U_LEFT + P_RIGHT) / 2

// The lowercase u that round 6 drew, kept here because the master no longer has
// it — asking word.mjs for a glyph it does not carry throws, which is the point.
const LOWER_U = `<path d="M120 ${W.xTop} V35 A15 15 0 0 0 150 35"/>
    <path d="M150 ${W.xTop} V${W.baseline}"/>`

// --- the arrow ----------------------------------------------------------------
// Unchanged from round 7 except for the chord, which is the question. The shaft
// is absolute and does not scale with length: the word is a monoline, every
// letter is 8 units and its one existing accent — the i's tittle — is an 8-unit
// circle, so an accent at any other weight is a second voice.
const ARROW_W = W.stroke
const HEAD = { half: ARROW_W * 1.4, long: ARROW_W * 1.7 }
const BOW = 0.72
const RISE = 0.43

const arrowBody = ({ run, rise }) => {
  const tail = { x: 0, y: 0 }
  const ctrl = { x: run * BOW, y: 0 }
  const neck = { x: run, y: -rise }
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

// An arrow of this shape, centred on `over`, lowest ink on `bottom`. The box is
// measured off the raster: depending on the rise the widest point is the tip or
// a head corner, and neither is worth working out by hand.
const arrowOver = ({ over, bottom, run, rise }) => {
  const body = arrowBody({ run, rise })
  const box = contentBox(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="-60 -140 280 200">${body}</svg>`)
  const tx = over - (box.x + box.w / 2)
  const ty = bottom - (box.y + box.h)
  return {
    run,
    rise,
    width: box.w,
    height: box.h,
    bottom,
    top: ty + box.y,
    left: tx + box.x,
    right: tx + box.x + box.w,
    draw: (accent, dx = 0) =>
      `<g transform="translate(${(tx + dx).toFixed(2)} ${ty.toFixed(2)})">${body.replaceAll('#FF5A2B', accent)}</g>`,
  }
}

// --- how much air is left between the arrow and the fire ----------------------
// The two ember objects are now at opposite ends of a lockup that got shorter,
// and the big arc reaches back over the word toward the mark. Whether they
// touch is not something to judge by eye on a sheet, so it is measured: render
// each on its own through the same viewBox, then take the distance from every
// pixel of one to the nearest pixel of the other.
//
// A two-pass chamfer transform, weights 1 and sqrt(2) — within a few percent of
// true Euclidean, which is far inside what matters here. 0 means they overlap.
const inkGap = (viewBox, a, b, width = 1400) => {
  const [, , vbW] = viewBox.trim().split(/[\s,]+/).map(Number)
  const layer = (body) => {
    const { pixels, width: w, height: h } = new Resvg(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}">${body}</svg>`,
      { fitTo: { mode: 'width', value: width } },
    ).render()
    const ink = new Uint8Array(w * h)
    for (let i = 0; i < w * h; i += 1) ink[i] = pixels[i * 4 + 3] > 8 ? 1 : 0
    return { ink, w, h }
  }
  const A = layer(a)
  const B = layer(b)
  if (A.w !== B.w || A.h !== B.h) throw new Error('inkGap: layers disagree on size')
  const { w, h } = A
  const FAR = 1e9
  const dist = new Float64Array(w * h)
  for (let i = 0; i < w * h; i += 1) dist[i] = A.ink[i] ? 0 : FAR
  const D = Math.SQRT2
  const relax = (at, from, cost) => {
    const candidate = dist[from] + cost
    if (candidate < dist[at]) dist[at] = candidate
  }
  for (let y = 0; y < h; y += 1) {
    for (let x = 0; x < w; x += 1) {
      const at = y * w + x
      if (x > 0) relax(at, at - 1, 1)
      if (y > 0) relax(at, at - w, 1)
      if (y > 0 && x > 0) relax(at, at - w - 1, D)
      if (y > 0 && x < w - 1) relax(at, at - w + 1, D)
    }
  }
  for (let y = h - 1; y >= 0; y -= 1) {
    for (let x = w - 1; x >= 0; x -= 1) {
      const at = y * w + x
      if (x < w - 1) relax(at, at + 1, 1)
      if (y < h - 1) relax(at, at + w, 1)
      if (y < h - 1 && x < w - 1) relax(at, at + w + 1, D)
      if (y < h - 1 && x > 0) relax(at, at + w - 1, D)
    }
  }
  let best = FAR
  for (let i = 0; i < w * h; i += 1) if (B.ink[i] && dist[i] < best) best = dist[i]
  return (best * vbW) / w // px -> viewBox units
}

// --- compositions -------------------------------------------------------------
const PAINT = 'fill="none" stroke="#FFFFFF" stroke-width="8" stroke-linecap="butt" stroke-linejoin="round"'
const drawn = (markup, fg, accent, shift = 0) =>
  `<g ${recolour(PAINT, fg, accent)} transform="translate(${shift.toFixed(2)} 0)">${markup}</g>`

// An option is the frozen mark, the word, and an arrow. `cap: false` falls back
// to round 6's lowercase u, for the comparison row only.
const compose = ({ cap = true, arrow = null }) => (fg, accent) => {
  const letters = cap ? ['i', 'r', 'e', 'U', 'p', 'tittle'] : ['i', 'r', 'e', 'p', 'tittle']
  const body = [MARK_PLACED(fg, accent), spell(letters, { shift: SHIFT, fg, accent })]
  if (!cap) body.push(drawn(LOWER_U, fg, accent, SHIFT))
  if (arrow) body.push(arrow.draw(accent, SHIFT))
  return body.join('\n  ')
}

// The ceiling every option works under: 3 units above the capital's top edge,
// which is the gap the tittle keeps over the i's stem. 7a A, in one line.
const CEILING = CAP_TOP - ACCENT_CLEARANCE

const CHORD_UP = P_RIGHT - U_LEFT - W.stroke // 76 — stem edge to stem edge across "Up"
const CHORD_OUT = 48 // 7c C's "out to the p"
const CHORD_U = U_RIGHT - U_LEFT - W.stroke // 30 — round 7, stem to stem on the U

const A_BIG = arrowOver({ over: MID_UP, bottom: CEILING, run: CHORD_UP, rise: CHORD_UP * RISE })
const B_MID = arrowOver({ over: MID_UP, bottom: CEILING, run: CHORD_OUT, rise: CHORD_OUT * RISE })
const C_OUT = arrowOver({ over: MID_U, bottom: CEILING, run: CHORD_OUT, rise: CHORD_OUT * RISE })
const D_NOW = arrowOver({ over: MID_U, bottom: CEILING, run: CHORD_U, rise: CHORD_U * RISE })

const OPTIONS = [
  {
    id: 'big',
    label: 'A · 7b row D — chord 76, the whole "Up"',
    arrow: A_BIG,
    note:
      'The answer, drawn: the arc spans from the\n' +
      "U's left edge to the end of the p's bowl,\n" +
      'centred on "Up". The gesture rather than\n' +
      'the accent, which is what was asked for.',
  },
  {
    id: 'mid',
    label: 'B · chord 48 over "Up" — the note\'s size, the answer\'s placement',
    arrow: B_MID,
    note:
      "7c C's 48-unit chord, but centred over\n" +
      '"Up" the way row D is. Where the two\n' +
      'answers would meet if the smaller chord\n' +
      'is what was meant by "out to the p".',
  },
  {
    id: 'out',
    label: 'C · 7c C — chord 48, centred over the U',
    arrow: C_OUT,
    note:
      'The note read literally: 48 across, still\n' +
      'centred on the U, so it overhangs the p on\n' +
      'one side and the e on the other.',
  },
  {
    id: 'now',
    label: 'D · Round 7 — chord 30, the accent (today)',
    arrow: D_NOW,
    note: 'What is on the PR now, for comparison.',
  },
]

// --- diagnostics --------------------------------------------------------------
const WIDE = '-80 -140 520 260'
const ROUND7 = contentBox(
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${WIDE}">${compose({ arrow: D_NOW })(PAPER, EMBER)}</svg>`,
)

const framed = (option, fg, accent) => {
  const body = option.body ? option.body(fg, accent) : compose({ arrow: option.arrow })(fg, accent)
  const box = contentBox(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="${WIDE}">${body}</svg>`)
  return {
    box,
    body,
    svg: (bg) =>
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${box.x.toFixed(2)} ${box.y.toFixed(2)} ${box.w.toFixed(2)} ${box.h.toFixed(2)}">` +
      (bg
        ? `<rect x="${box.x.toFixed(2)}" y="${box.y.toFixed(2)}" width="${box.w.toFixed(2)}" height="${box.h.toFixed(2)}" fill="${bg}"/>`
        : '') +
      `${body}</svg>`,
  }
}

const MIN = 96 // the stated lockup minimum, in px wide
const FIRE_TIP = markY(M.fireTip)

// The wordmark master's own viewBox — the arrow has to fit inside it or the
// master draws outside what it declares, which smoke.mjs now fails on.
const WORDMARK_VB = master('wordmark').match(/viewBox="([^"]+)"/)[1].trim().split(/[\s,]+/).map(Number)

console.log(`accent clearance, read off the master: ${ACCENT_CLEARANCE} units`)
console.log(`capital top edge y=${CAP_TOP}, so the arrow's ceiling is y=${CEILING}`)
console.log(`chords: "Up" ${CHORD_UP}, 7c C ${CHORD_OUT}, round 7 ${CHORD_U}`)
console.log(`the mark's fire tips at y=${FIRE_TIP.toFixed(1)} in lockup units`)
console.log(`round 7 envelope: ${ROUND7.w.toFixed(1)} x ${ROUND7.h.toFixed(1)}, top y=${ROUND7.y.toFixed(1)}`)
console.log(`wordmark master viewBox: ${WORDMARK_VB.join(' ')}\n`)

console.table(
  OPTIONS.map((option) => {
    const { box } = framed(option, PAPER, EMBER)
    const px = MIN / box.w // one lockup unit, in px, at the 96px minimum
    const a = option.arrow
    const gap = inkGap(
      WIDE,
      FIRE_ONLY,
      `<g transform="translate(${SHIFT.toFixed(2)} 0)">${a.draw(EMBER)}</g>`,
    )
    return {
      id: option.id,
      chord: a.run,
      aspect: `${(box.w / box.h).toFixed(2)}:1`,
      'arrow box': `${a.width.toFixed(1)}x${a.height.toFixed(1)}`,
      'clears cap by': (CAP_TOP - a.bottom).toFixed(1),
      'vs fire tip': a.top < FIRE_TIP ? `${(FIRE_TIP - a.top).toFixed(1)} above` : `${(a.top - FIRE_TIP).toFixed(1)} below`,
      'air to fire': gap.toFixed(1),
      'fits wordmark vb': a.top >= WORDMARK_VB[1] && a.right <= WORDMARK_VB[0] + WORDMARK_VB[2] ? 'yes' : 'NO — widen it',
      'at 96px': `${(a.width * px).toFixed(1)}x${(a.height * px).toFixed(1)}px, shaft ${(ARROW_W * px).toFixed(1)}px`,
      'x-height at 96px': ((W.baseline - W.xTop) * px).toFixed(1),
    }
  }),
)

// --- the sheets ---------------------------------------------------------------
const FONT = 'DejaVu Sans, sans-serif'
const WIDTHS = [320, 200, 128, 96]
const PAD = 28
const ROW_GAP = 34
const NOTE_COL = 330

const text = (x, y, body, { size = 15, fill = PAPER, weight = 'bold' } = {}) =>
  `<text x="${x}" y="${y}" font-family="${FONT}" font-size="${size}" font-weight="${weight}" fill="${fill}">${body}</text>`

const sheetOf = ({ options, fg, accent, bg, muted, caption }) => {
  const rows = options.map((option) => framed(option, fg, accent))
  const width = PAD * 2 + WIDTHS.reduce((sum, w) => sum + w + 28, 0) + NOTE_COL
  let y = PAD + 30
  const blocks = options
    .map((option, index) => {
      const row = rows[index]
      const rowH = (WIDTHS[0] * row.box.h) / row.box.w
      const top = y
      let x = PAD
      const tiles = WIDTHS.map((w) => {
        const h = (w * row.box.h) / row.box.w
        const tile = `<svg viewBox="${row.box.x.toFixed(2)} ${row.box.y.toFixed(2)} ${row.box.w.toFixed(2)} ${row.box.h.toFixed(2)}" x="${x}" y="${(top + 22 + (rowH - h) / 2).toFixed(2)}" width="${w}" height="${h.toFixed(2)}">${row.body}</svg>`
        x += w + 28
        return tile
      }).join('')
      const notes = option.note
        .split('\n')
        .map((line, i) => text(x + 6, top + 36 + i * 17, line, { size: 13, fill: muted, weight: 'normal' }))
        .join('')
      y = top + 22 + rowH + ROW_GAP
      return `${text(PAD, top + 12, option.label, { fill: fg })}${tiles}${notes}`
    })
    .join('')

  const height = y + 30
  let labelX = PAD
  const scale = WIDTHS.map((w) => {
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

// 96px is where opinions change and too small to argue about by looking: render
// each option at 96 wide, read the renderer's own pixels back, redraw them as
// one square per pixel.
const zoomSheet = ({ options, fg, accent, bg, muted, caption, width: px = MIN, scale = 5 }) => {
  const rows = options.map((option) => framed(option, fg, accent))
  let y = PAD + 16
  const blocks = options
    .map((option, index) => {
      const row = rows[index]
      const h = Math.round((px * row.box.h) / row.box.w)
      const top = y
      y = top + 24 + h * scale + 30
      return `${text(PAD, top + 12, option.label, { fill: fg })}
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

const out = join(root, 'gesture')
mkdirSync(out, { recursive: true })

const DARK = { fg: PAPER, accent: EMBER, bg: INK, muted: MUTED }
const LIGHT = { fg: INK, accent: EMBER_DEEP, bg: PAPER, muted: '#5A6782' }

for (const config of [
  {
    file: 'chord-dark',
    options: OPTIONS,
    ...DARK,
    caption: `Round 8a — how long the arc is   ·   on ink, ember ${EMBER}   ·   the mark is unchanged in all four`,
  },
  {
    file: 'chord-light',
    options: OPTIONS,
    ...LIGHT,
    caption: `Round 8a — how long the arc is   ·   on paper, ember-deep ${EMBER_DEEP}   ·   the mark is unchanged in all four`,
  },
]) {
  const source = sheetOf(config)
  const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const file = join(out, `${config.file}.png`)
  writeFileSync(file, renderPng(source, width))
  console.log(file)
}

const zoom = zoomSheet({
  options: OPTIONS,
  ...DARK,
  caption: 'Round 8a at 96px wide, magnified 5x — real pixels, no resampling',
})
writeFileSync(join(out, 'zoom-chord.png'), renderPng(zoom, Number(zoom.match(/viewBox="0 0 (\d+)/)[1])))
console.log(join(out, 'zoom-chord.png'))

// --- what goes into the master -------------------------------------------------
// The shipped option's geometry, in the wordmark's own coordinates — the lockup
// shifts the word right by SHIFT and that shift is not part of the drawing.
// Transcribed into logo/wordmark.svg by hand, which is what keeps the master a
// drawing someone can read and correct; printed here so it is transcribed once
// rather than worked out twice.
const SHIP = A_BIG
console.log(
  `\n8a-A — chord ${SHIP.run}x${(SHIP.run * RISE).toFixed(2)}, shaft ${ARROW_W}, box ` +
    `${SHIP.width.toFixed(1)}x${SHIP.height.toFixed(1)} on x=${SHIP.left.toFixed(1)}..${SHIP.right.toFixed(1)}, ` +
    `y=${SHIP.top.toFixed(1)}..${SHIP.bottom.toFixed(1)}, in wordmark coordinates:`,
)
console.log(SHIP.draw('#FF5A2B').replaceAll('/><', '/>\n  '))

// Each option on its own at the two widths that decide it: 96, the stated
// minimum, and 960, where a head that does not quite point shows up.
for (const option of OPTIONS) {
  const row = framed(option, PAPER, EMBER)
  for (const w of [MIN, 960]) {
    const file = join(out, `${option.id}-${w}.png`)
    writeFileSync(file, renderPng(row.svg(INK), w))
    console.log(file)
  }
}
