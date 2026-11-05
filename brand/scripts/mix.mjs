#!/usr/bin/env node
// Round six. The CEO's note on round 5: "A is perfect. Mix this F into fireup
// instead of locating next." So 5a-A is the mark, frozen, and this round is
// about the *lockup* only: the F stops standing next to the word and becomes
// the word's initial letter.
//
//   node scripts/mix.mjs   ->  brand/mix/*.png
//
// Nothing here redraws the mark. mark.svg is read as-is and placed; the only
// variables are how big it sits relative to the lowercase and where the rest of
// the word starts. The word's own monoline `f` is dropped, because the mark is
// now that letter.
//
// The one hard geometric problem, measured rather than guessed:
//
//   The approved mark's top bar runs to x=36 and the fire reaches x=56 — half
//   the width of the mark sits in the band *above* lowercase x-height. That
//   band is exactly where the i's tittle lives. So "ireup" cannot simply be
//   tucked under the bar at the same size: the dot of the i lands inside the
//   fire. Each option below answers that collision differently, and the script
//   prints the real clearance for each so the answer is not eyeballed.
//
// Wordmark construction (logo/wordmark.svg): baseline y=50, x-height top y=20,
// ascender top y=4, descender y=66, stroke 8. "ireup" spans x=30..200.
// Mark construction (logo/mark.svg): F foot y=60, cap top y=16 (cap height 44),
// bar weight 12, middle arm ends x=28, bar ends x=36, fire tip y=1, fire out
// to x=56.
import { Resvg } from '@resvg/resvg-js'
import { renderPng, contentBox, pixelMap } from './render.mjs'
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

const master = (name) => readFileSync(join(root, 'logo', `${name}.svg`), 'utf8')
const recolour = (svg, fg, accent) => svg.replaceAll('#FFFFFF', fg).replaceAll('#FF5A2B', accent)
const innerOf = (svg) => svg.replace(/^[\s\S]*?<svg[^>]*>/, '').replace(/<\/svg>\s*$/, '')

// --- the two masters, taken apart only as far as this round needs -------------

const MARK = innerOf(master('mark'))

// The wordmark's letters in drawing order: the f is the first two paths, so
// "ireup" is everything after them. Read out of the master rather than retyped,
// so a change to the wordmark reaches these sheets.
const WORD = master('wordmark')
const WORD_PATHS = WORD.match(/<path [^>]*\/>/g)
const WORD_DOT = WORD.match(/<circle [^>]*\/>/)[0]
const IREUP = WORD_PATHS.slice(2).join('\n    ')
const F_PATHS = WORD_PATHS.slice(0, 2).join('\n    ')

// Geometry constants of the two drawings, named once.
const W = { baseline: 50, xTop: 20, ascTop: 4, desc: 66, stroke: 8, ireupL: 30, ireupR: 200 }
const M = { foot: 60, capTop: 16, bar: 12, armEnd: 28, barEnd: 36, fireTip: 1, fireEnd: 56, stemL: 6 }
const DOT = { cx: 34, cy: 9, r: 4 } // the i's tittle, in wordmark units

// The wordmark's own whitespace between letters, measured off the master: the
// i's stem spans x=30..38 and the r's spans 46..54, so eight units of air.
// I had this at 14 — the stem-to-stem pitch — until 6c was rendered.
const LETTER_GAP = 8

// Place the mark in wordmark coordinates: its foot on the word's baseline, its
// stem's left edge at `mx`, scaled by `s`.
const markAt = (s, mx) => ({
  s,
  mx,
  x: (u) => mx + (u - M.stemL) * s,
  y: (v) => W.baseline + (v - M.foot) * s,
  place: (fg, accent) =>
    `<g transform="translate(${(mx - M.stemL * s).toFixed(2)} ${(W.baseline - M.foot * s).toFixed(2)}) scale(${s.toFixed(4)})">${recolour(MARK, fg, accent)}</g>`,
})

// "ireup", moved so its left edge lands on `wx`. `tittle` false drops the dot.
const ireupAt = (wx, { tittle = true } = {}) => (fg) => `
    <g fill="none" stroke="${fg}" stroke-width="${W.stroke}" stroke-linecap="butt" stroke-linejoin="round"
       transform="translate(${(wx - W.ireupL).toFixed(2)} 0)">
    ${IREUP}
    </g>
    ${tittle ? `<circle cx="${(DOT.cx + wx - W.ireupL).toFixed(2)}" cy="${DOT.cy}" r="${DOT.r}" fill="${fg}"/>` : ''}`

// --- the options --------------------------------------------------------------
// Each is a composition in wordmark units. `clearance` is the vertical gap
// between the underside of the mark's overhanging bar/fire and the top of the
// i's tittle, where the two overlap horizontally — negative means they collide.

const MX = 0 // the F's stem sits on the left edge of every composition

const compose = (parts) => (fg, accent) => parts.map((part) => part(fg, accent)).join('\n')

// How close "ireup" can come before the i's tittle fouls the fire, measured by
// rendering rather than by bounding boxes. The fire is a leaning flame: its
// bounding box runs to x=56 but there is open air under its outer tongue, and
// a box test throws that away. So: grow the tittle by the clearance we want,
// render it and the mark into the same bitmap, and look for a pixel they both
// cover. Binary search the smallest wx with no such pixel.
const PROBE = { x: -20, y: -60, w: 200, h: 140, px: 600 }
const probeMap = (body) => {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${PROBE.x} ${PROBE.y} ${PROBE.w} ${PROBE.h}">${body}</svg>`
  const { pixels, width, height } = new Resvg(svg, { fitTo: { mode: 'width', value: PROBE.px } }).render()
  const alpha = new Uint8Array(width * height)
  for (let i = 0; i < alpha.length; i += 1) alpha[i] = pixels[i * 4 + 3] > 8 ? 1 : 0
  return { alpha, width, height }
}

const tightestWx = (mark, clearance) => {
  const ink = probeMap(mark.place('#FFFFFF', '#FFFFFF'))
  const unit = PROBE.w / ink.width
  const hits = (wx) => {
    const cx = wx + (DOT.cx - W.ireupL)
    const r = DOT.r + clearance
    const px = (v) => Math.round((v - PROBE.x) / unit)
    const py = (v) => Math.round((v - PROBE.y) / unit)
    const rp = Math.round(r / unit)
    for (let dy = -rp; dy <= rp; dy += 1) {
      const half = Math.floor(Math.sqrt(Math.max(0, rp * rp - dy * dy)))
      const row = py(DOT.cy) + dy
      if (row < 0 || row >= ink.height) continue
      for (let dx = -half; dx <= half; dx += 1) {
        const col = px(cx) + dx
        if (col < 0 || col >= ink.width) continue
        if (ink.alpha[row * ink.width + col]) return true
      }
    }
    return false
  }
  let lo = mark.x(M.stemL)
  let hi = mark.x(M.fireEnd) + 20
  for (let i = 0; i < 14; i += 1) {
    const mid = (lo + hi) / 2
    if (hits(mid)) lo = mid
    else hi = mid
  }
  return hi
}

// A · Inline — the mark at the word's own ascender height, the word as close as
// the tittle's clearance allows. The clearance is the wordmark's own smallest
// counter-gap (stem to stem 22, stroke 8), so the tittle is no tighter to the
// fire than the letters are to each other.
const A_MARK = markAt((W.baseline - W.ascTop) / (M.foot - M.capTop), MX)
const A_WX = Math.round(tightestWx(A_MARK, 6) * 10) / 10
const INLINE = {
  id: 'inline',
  label: 'A · Inline — one size, word clears the fire',
  mark: A_MARK,
  wx: A_WX,
  tittle: true,
  note:
    "The F is exactly as tall as the word's own\n" +
    'ascender. Reads as one logotype at 320. But\n' +
    'the fire and the i\'s tittle end up side by\n' +
    'side with nothing between them: by 128px the\n' +
    'two merge into one blob, and at 96 the word\n' +
    'looks like it starts with a smudge.',
}

// B · Nested — the mark used as an initial, big enough that the whole bar and
// fire ride above the word's ascender, and "ireup" tucked in right after the
// F's middle arm.
const B_S = 1.5
const B_MARK = markAt(B_S, MX)
const B_WX = B_MARK.x(M.armEnd) + LETTER_GAP
const NESTED = {
  id: 'nested',
  label: 'B · Nested — the F is the initial, word tucks in',
  mark: B_MARK,
  wx: B_WX,
  tittle: true,
  note:
    'The F is 1.5x the lowercase, so the bar and\n' +
    'the fire both ride clear above the ascender\n' +
    'and the word starts right after the middle\n' +
    'arm. The fire has air around it at every\n' +
    'size, which is what A loses. Ships; tuned in\n' +
    '6b and 6c.',
}

// C · Ligature — the mark at ascender height, "ireup" tucked hard against the
// stem, and the i's tittle dropped: its stem runs up to meet the bar instead,
// so the bar is what dots the i.
const C_MARK = markAt(A_MARK.s, MX)
const C_WX = C_MARK.x(M.armEnd) + LETTER_GAP
const LIGATURE = {
  id: 'ligature',
  label: 'C · Ligature — the bar dots the i',
  mark: C_MARK,
  wx: C_WX,
  tittle: false,
  riser: true,
  note:
    'The tightest mix: the i loses its dot and\n' +
    "runs up into the bar's underside. It does not\n" +
    'read as an i. An undotted stem that tall is\n' +
    'an l, so the word reads "Flreup" — at 320px\n' +
    'as clearly as at 96. Rejected.',
}

// D · Round 5, for reference — the mark standing next to the whole word, which
// is what is on the PR today and what the note asked to replace.
const D_MARK = markAt(A_MARK.s, MX)
const D_WX = D_MARK.x(M.fireEnd) + 20
const BESIDE = {
  id: 'beside',
  label: 'D · Round 5 — mark beside the word (today)',
  mark: D_MARK,
  wx: D_WX,
  tittle: true,
  keepF: true,
  note:
    'What is on the PR now, for comparison: the\n' +
    'mark, a clear-space gap, then the complete\n' +
    'word including its own f. Two objects, one\n' +
    'of which says the name twice.',
}

const DIRECTIONS = [INLINE, NESTED, LIGATURE, BESIDE]

// --- 6b: how big the F is ------------------------------------------------------
// With the nested direction chosen, the F's size is not a taste decision first:
// it is what buys the tittle its clearance. The bar's underside sits at
// y = 50 - 32s and the tittle's top at y = 5, so the clearance is 32s - 45.
// Below s = 1.41 the dot touches the fire and the direction collapses back into
// option A. The three below are the dot grazing, a 3-unit gap, and a 6-unit gap.
const TUCK = LETTER_GAP // held constant here; 6c varies it
const sizeOption = (s, id, label, note) => {
  const mark = markAt(s, MX)
  return { id, label, mark, wx: mark.x(M.armEnd) + TUCK, tittle: true, note }
}

const SIZES = [
  sizeOption(
    1.41,
    'size-141',
    "A · Cap 1.35x the word's ascender",
    'The smallest F that lets the word tuck\n' +
      'under the bar at all. Clearance 0.1 units:\n' +
      'at 96px the dot and the fire land on the\n' +
      'same pixel and the i loses its tittle.',
  ),
  sizeOption(
    1.5,
    'size-150',
    'B · Cap 1.43x',
    'Clearance 3 units. At 96px, magnified, the\n' +
      'dot is a separate pixel with dark around\n' +
      'it. Ships.',
  ),
  sizeOption(
    1.6,
    'size-160',
    'C · Cap 1.53x — more initial',
    'Clearance 6.2 units, which buys nothing the\n' +
      '3 did not already buy, and the F is big\n' +
      'enough to read as a drop cap with a word\n' +
      'after it rather than the first letter of one.',
  ),
]

// --- 6c: how tight the word tucks ----------------------------------------------
// The word cannot start before the F's middle arm ends — below that the arm and
// the x-height band of the lowercase occupy the same 9 units of height. So the
// only variable is the gap after the arm. The wordmark's own letters sit 14
// units apart at x-height, which is the middle option.
const TUCK_S = 1.5 // set from 6b after looking at it
const tuckOption = (gap, id, label, note) => {
  const mark = markAt(TUCK_S, MX)
  return { id, label, mark, wx: mark.x(M.armEnd) + gap, tittle: true, note }
}

const TUCKS = [
  tuckOption(
    LETTER_GAP,
    'tuck-8',
    "A · Gap 8 — the word's own letter gap",
    'Exactly the air the word already keeps\n' +
      'between its i and its r, so the F is spaced\n' +
      'like a letter and not like a mark. Reads as\n' +
      'one word at every width. Ships.',
  ),
  tuckOption(14, 'tuck-14', 'B · Gap 14 — one bar of the mark', 'Legible, but the F begins to stand off.'),
  tuckOption(
    20,
    'tuck-20',
    'C · Gap 20 — looser',
    'The hole under the middle arm opens up and\n' + 'the lockup goes back to being two objects.',
  ),
]

// The i's riser for option C: the stem, extended from the baseline up to the
// underside of the bar so the two meet on a seam.
const riserOf = (option) => (fg) => {
  const top = option.mark.y(28)
  const cx = option.wx + (DOT.cx - W.ireupL)
  return `<rect x="${(cx - W.stroke / 2).toFixed(2)}" y="${top.toFixed(2)}" width="${W.stroke}" height="${(W.baseline - top).toFixed(2)}" fill="${fg}"/>`
}

const bodyOf = (option) => {
  const parts = [option.mark.place]
  if (option.keepF) {
    parts.push((fg) => `<g fill="none" stroke="${fg}" stroke-width="${W.stroke}" stroke-linecap="butt" stroke-linejoin="round"
       transform="translate(${(option.wx - W.ireupL).toFixed(2)} 0)">
    ${F_PATHS}
    </g>`)
  }
  parts.push(ireupAt(option.wx, { tittle: option.tittle }))
  if (option.riser) parts.push(riserOf(option))
  return compose(parts)
}

// Tight viewBox from the rendered alpha, the same way build.mjs measures the
// masters — a hand-counted box is what clipped round 4's flame.
const framed = (option, fg, accent) => {
  const body = bodyOf(option)(fg, accent)
  const loose = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="-60 -80 420 200">${body}</svg>`
  const box = contentBox(loose)
  return {
    box,
    svg: (bg) =>
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${box.x.toFixed(2)} ${box.y.toFixed(2)} ${box.w.toFixed(2)} ${box.h.toFixed(2)}">` +
      (bg ? `<rect x="${box.x.toFixed(2)}" y="${box.y.toFixed(2)}" width="${box.w.toFixed(2)}" height="${box.h.toFixed(2)}" fill="${bg}"/>` : '') +
      `${body}</svg>`,
    body,
  }
}

// --- diagnostics --------------------------------------------------------------
// Printed, not eyeballed: the tittle clearance and the resulting aspect ratio.
const FONT = 'DejaVu Sans, sans-serif'
const ALL = [...DIRECTIONS, ...SIZES, ...TUCKS]
const diagnostics = ALL.map((option) => {
  const dotTop = DOT.cy - DOT.r
  const underside = option.mark.y(28)
  const dotL = option.wx + (DOT.cx - W.ireupL) - DOT.r
  const dotR = dotL + DOT.r * 2
  const under = option.tittle && dotR > option.mark.x(M.stemL) && dotL < option.mark.x(M.fireEnd)
  const clear = under ? dotTop - underside : null
  const { box } = framed(option, PAPER, EMBER)
  return {
    id: option.id,
    capRatio: ((option.mark.s * (M.foot - M.capTop)) / (W.baseline - W.ascTop)).toFixed(2),
    barVsStroke: (option.mark.s * M.bar).toFixed(1),
    tittleUnderOverhang: under,
    tittleClearance: clear === null ? 'n/a' : clear.toFixed(1),
    aspect: `${(box.w / box.h).toFixed(2)}:1`,
    at96: { xHeight: ((30 * 96) / box.w).toFixed(1), stroke: ((8 * 96) / box.w).toFixed(1) },
  }
})
console.table(diagnostics.map((d) => ({ ...d, at96: `x-h ${d.at96.xHeight}px / stroke ${d.at96.stroke}px` })))

// --- the sheet ----------------------------------------------------------------
// A lockup is wide, so each option is a row and the sizes run across it. The
// widths are the ones that matter: the site header (200), a doc or README (320),
// a dense UI row (128) and the stated 96px minimum.
const WIDTHS = [320, 200, 128, 96]
const PAD = 28
const ROW_GAP = 34
const NOTE_COL = 330

const text = (x, y, body, { size = 15, fill = PAPER, weight = 'bold' } = {}) =>
  `<text x="${x}" y="${y}" font-family="${FONT}" font-size="${size}" font-weight="${weight}" fill="${fill}">${body}</text>`

const sheetOf = ({ options, fg, accent, bg, muted, caption }) => {
  const rows = options.map((option) => framed(option, fg, accent))
  const rowHeights = rows.map((row) => (WIDTHS[0] * row.box.h) / row.box.w)
  const width = PAD * 2 + WIDTHS.reduce((sum, w) => sum + w + 28, 0) + NOTE_COL
  let y = PAD + 30
  const blocks = options.map((option, index) => {
    const row = rows[index]
    const rowH = rowHeights[index]
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
  }).join('')

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

// The 96px column decides this round, and 96px is too small to argue about by
// looking at it. This sheet renders each option at 96 wide, reads the
// renderer's own pixels back, and redraws them as one square per pixel — so
// what is on screen cannot disagree with what ships.
const zoomSheet = ({ options, fg, accent, bg, muted, caption, width: px = 96, scale = 5 }) => {
  const rows = options.map((option) => framed(option, fg, accent))
  let y = PAD + 16
  const blocks = options.map((option, index) => {
    const row = rows[index]
    const h = Math.round((px * row.box.h) / row.box.w)
    const top = y
    y = top + 24 + h * scale + 30
    return `${text(PAD, top + 12, option.label, { fill: fg })}
      <g transform="translate(${PAD} ${top + 24})">${pixelMap(row.svg(bg), px, scale)}</g>`
  }).join('')
  const width = PAD * 2 + px * scale + 40
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${y + 24}" width="${width}" height="${y + 24}">
  <rect width="${width}" height="${y + 24}" fill="${bg}"/>
  ${blocks}
  ${text(PAD, y + 8, caption, { size: 13, fill: muted, weight: 'normal' })}
</svg>`
}

const out = join(root, 'mix')
mkdirSync(out, { recursive: true })

const DARK = { fg: PAPER, accent: EMBER, bg: INK, muted: MUTED }
const LIGHT = { fg: INK, accent: EMBER_DEEP, bg: PAPER, muted: '#5A6782' }

const SHEETS = [
  {
    file: 'mix-dark',
    options: DIRECTIONS,
    ...DARK,
    caption:
      'Round 6a — mixing the approved F into the word   ·   on ink, ember #FF5A2B   ·   the mark itself is unchanged in all four',
  },
  {
    file: 'mix-light',
    options: DIRECTIONS,
    ...LIGHT,
    caption:
      'Round 6a — mixing the approved F into the word   ·   on paper, ember-deep #C2410C   ·   the mark itself is unchanged in all four',
  },
  {
    file: 'size-dark',
    options: SIZES,
    ...DARK,
    caption: 'Round 6b — how big the F is, nested direction   ·   on ink   ·   below 1.41x the tittle fouls the fire',
  },
  {
    file: 'size-light',
    options: SIZES,
    ...LIGHT,
    caption: 'Round 6b — how big the F is, nested direction   ·   on paper   ·   below 1.41x the tittle fouls the fire',
  },
  {
    file: 'tuck-dark',
    options: TUCKS,
    ...DARK,
    caption: `Round 6c — how tight the word tucks, F at ${TUCK_S}x   ·   on ink   ·   the word cannot start before the middle arm ends`,
  },
  {
    file: 'tuck-light',
    options: TUCKS,
    ...LIGHT,
    caption: `Round 6c — how tight the word tucks, F at ${TUCK_S}x   ·   on paper   ·   the word cannot start before the middle arm ends`,
  },
]

for (const config of SHEETS) {
  const source = sheetOf(config)
  const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const file = join(out, `${config.file}.png`)
  writeFileSync(file, renderPng(source, width))
  console.log(file)
}

for (const [file, options, caption] of [
  ['zoom-directions', DIRECTIONS, 'Round 6a at 96px wide, magnified 5x — real pixels, no resampling'],
  ['zoom-size', SIZES, 'Round 6b at 96px wide, magnified 5x — real pixels, no resampling'],
  ['zoom-tuck', TUCKS, 'Round 6c at 96px wide, magnified 5x — real pixels, no resampling'],
]) {
  const source = zoomSheet({ options, ...DARK, caption })
  const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const path = join(out, `${file}.png`)
  writeFileSync(path, renderPng(source, width))
  console.log(path)
}

// Each option on its own at the two widths that decide it: 96, the stated
// minimum, and 960, where a join that does not quite meet shows up.
for (const option of ALL) {
  const row = framed(option, PAPER, EMBER)
  for (const w of [96, 960]) {
    const file = join(out, `${option.id}-${w}.png`)
    writeFileSync(file, renderPng(row.svg(INK), w))
    console.log(file)
  }
}
