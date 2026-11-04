#!/usr/bin/env node
// Renders the four cheap tuning axes on the chosen direction, one sheet per
// axis, so a "changes needed" note can be given by pointing at an option
// instead of writing a brief.
//
//   node scripts/tune.mjs   ->  brand/tune/*.png
//
// Each sheet shows every option at the sizes it will really be used at, and
// for the mark it also shows the actual 16px raster magnified 8x with no
// resampling — the pixels that landed, not a smooth preview of them.
import { renderPng, pixelMap } from './render.mjs'
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')
const tokens = JSON.parse(readFileSync(join(root, 'tokens', 'brand.json'), 'utf8'))

const INK = tokens.color.ink
const PAPER = tokens.color.paper
const EMBER = tokens.color.ember

// --- geometry, parameterised on the two axes that move the drawing ----------

// `gap` is the clear space between the stem's right edge and the lifted arm.
// The arm's right edge is pinned at x=52 to keep the mark optically centred, so
// a wider gap necessarily buys a shorter arm — that trade is part of the choice.
const mark = ({ gap = 6, armY = 27, fg = PAPER, accent = EMBER } = {}) => {
  const armX = 24 + gap
  return `
    <rect x="12" y="9" width="12" height="46" fill="${fg}"/>
    <rect x="12" y="9" width="36" height="12" fill="${fg}"/>
    <rect x="${armX}" y="${armY}" width="${52 - armX}" height="12" fill="${accent}"/>`
}

// The wordmark is monoline, so weight is one number. The i-dot is drawn solid
// and scales with the stroke so it keeps its weight at small sizes.
const wordmark = ({ weight = 9, fg = PAPER } = {}) => `
  <g fill="none" stroke="${fg}" stroke-width="${weight}" stroke-linecap="butt" stroke-linejoin="round">
    <path d="M12 50 V13 A8 8 0 0 1 20 5"/>
    <path d="M4 20 H23"/>
    <path d="M34 20 V50"/>
    <path d="M50 20 V50"/>
    <path d="M50 32 Q50 20 63 20"/>
    <path d="M104 35 A15 15 0 1 0 96.5 48"/>
    <path d="M74 35 H104"/>
    <path d="M120 20 V35 A15 15 0 0 0 150 35"/>
    <path d="M150 20 V50"/>
    <path d="M166 20 V66"/>
    <path d="M166 20 H181 A15 15 0 0 1 181 50 H166"/>
  </g>
  <circle cx="34" cy="9" r="${(4.5 * weight) / 9}" fill="${fg}"/>`

// The 16px magnified raster comes from `pixelMap` in render.mjs — real pixels,
// no decoder and no resampler, so the blow-up cannot disagree with what ships.

// --- the axes ---------------------------------------------------------------

const MARK_SIZES = [96, 48, 32, 16]
const WORD_WIDTHS = [240, 160, 96]

const AXES = [
  {
    id: 'ember',
    title: 'Axis 1 — the ember colour',
    question: 'Which accent?',
    kind: 'mark',
    options: [
      {
        id: 'warmer',
        label: 'A · warmer',
        note: '#FF8A2B · 7.95:1 on ink\nBrightest of the three. Also the closest\nto fireup.pro’s black-and-yellow.',
        draw: () => mark({ accent: '#FF8A2B' }),
      },
      {
        id: 'current',
        label: 'B · as drawn',
        note: '#FF5A2B · 6.02:1 on ink\nReads as heat without being a warning\ncolour. Furthest from amber status UI.',
        draw: () => mark({ accent: EMBER }),
      },
      {
        id: 'cooler',
        label: 'C · cooler',
        note: '#FF4D5E · 5.77:1 on ink\nMore distinctive, least “hot”. Risks\nreading as an error state.',
        draw: () => mark({ accent: '#FF4D5E' }),
      },
    ],
  },
  {
    id: 'gap',
    title: 'Axis 2 — the gap between the lifted arm and the stem',
    question: 'How far has the row travelled?',
    kind: 'mark',
    options: [
      {
        id: 'tight',
        label: 'A · 4 units',
        note: 'Still legibly an F. At 16px the gap is\n1px, so it nearly closes — see the\nmagnified raster.',
        draw: () => mark({ gap: 4 }),
      },
      {
        id: 'current',
        label: 'B · 6 units (as drawn)',
        note: '1.5px at 16px. The gap survives, and\nthe arm still belongs to the letter.',
        draw: () => mark({ gap: 6 }),
      },
      {
        id: 'wide',
        label: 'C · 10 units',
        note: 'The departure is unmistakable, but the\nF starts to come apart and the arm is\nshorter to pay for it.',
        draw: () => mark({ gap: 10 }),
      },
    ],
  },
  {
    id: 'counter',
    title: 'Axis 3 — the counter height (where the arm sits)',
    question: 'How high is the arm?',
    kind: 'mark',
    options: [
      {
        id: 'high',
        label: 'A · arm at 48%',
        note: 'Tight 4-unit counter. At 16px the arm\nfuses with the top bar — the counter is\ngone. Fails the size that matters most.',
        draw: () => mark({ armY: 25 }),
      },
      {
        id: 'current',
        label: 'B · arm at 52% (as drawn)',
        note: '6-unit counter, equal to the bar weight.\nEvery gap in the mark is one bar.',
        draw: () => mark({ armY: 27 }),
      },
      {
        id: 'low',
        label: 'C · arm at 61%',
        note: 'Open 10-unit counter. Reads as a gap\nleft behind, but weakens the F.',
        draw: () => mark({ armY: 31 }),
      },
    ],
  },
  {
    id: 'weight',
    title: 'Axis 4 — the wordmark weight',
    question: 'Which weight?',
    kind: 'wordmark',
    options: [
      {
        id: 'light',
        label: 'A · stroke 8',
        note: 'Counters stay widest open at 96px, so\nit is the most legible of the three at\nthe minimum size — but it reads light\nnext to the solid mark.',
        draw: () => wordmark({ weight: 8 }),
      },
      {
        id: 'current',
        label: 'B · stroke 9 (as drawn)',
        note: 'Matches the mark’s bar weight at the\nsame height, and the e and p counters\nstill hold at 96px.',
        draw: () => wordmark({ weight: 9 }),
      },
      {
        id: 'bold',
        label: 'C · stroke 11',
        note: 'Rendering disproves the obvious guess:\nheavier is worse small. The e’s aperture\nand the p’s bowl close up at 96px — see\nthe bottom row.',
        draw: () => wordmark({ weight: 11 }),
      },
    ],
  },
]

// --- sheet composition ------------------------------------------------------

const FONT = 'DejaVu Sans, sans-serif'
const label = (x, y, text, { size = 15, fill = PAPER, weight = 'bold' } = {}) =>
  `<text x="${x}" y="${y}" font-family="${FONT}" font-size="${size}" font-weight="${weight}" fill="${fill}">${text}</text>`

const note = (x, y, text) =>
  text
    .split('\n')
    .map((line, i) => label(x, y + i * 17, line, { size: 13, fill: tokens.color['paper-muted'], weight: 'normal' }))
    .join('')

const COL = 400
const PAD = 28

const markSheet = (axis) => {
  const rowTop = PAD + 56
  const sizesHeight = MARK_SIZES[0]
  const zoomTop = rowTop + sizesHeight + 36
  const zoom = 16 * 8
  const height = zoomTop + zoom + 112
  const width = PAD * 2 + COL * 3

  const columns = axis.options
    .map((option, index) => {
      const x = PAD + COL * index
      const body = option.draw()
      let tileX = x
      const tiles = MARK_SIZES.map((size) => {
        const tile = `<g transform="translate(${tileX} ${rowTop + (sizesHeight - size) / 2}) scale(${size / 64})">${body}</g>`
        tileX += size + 24
        return tile
      }).join('')
      const tiny = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="${INK}"/>${body}</svg>`
      return `
        ${label(x, PAD + 22, option.label)}
        ${tiles}
        ${label(x, zoomTop - 12, '16px, magnified 8x — real pixels', { size: 12, fill: tokens.color['paper-muted'], weight: 'normal' })}
        <g transform="translate(${x} ${zoomTop})">${pixelMap(tiny, 16, 8)}</g>
        ${note(x + zoom + 20, zoomTop + 18, option.note)}`
    })
    .join('')

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
  <rect width="${width}" height="${height}" fill="${INK}"/>
  ${label(PAD, height - 24, `${axis.title} — ${axis.question}   ·   96 / 48 / 32 / 16 px, actual size`, { size: 14, fill: tokens.color['paper-muted'], weight: 'normal' })}
  ${columns}
</svg>`
}

const wordmarkSheet = (axis) => {
  const width = PAD * 2 + COL * 3
  const rowTop = PAD + 56
  const lineHeight = 110
  const height = rowTop + lineHeight * WORD_WIDTHS.length + 148

  const columns = axis.options
    .map((option, index) => {
      const x = PAD + COL * index
      const body = option.draw()
      const lines = WORD_WIDTHS.map((w, row) => {
        const scale = w / 212
        return `<g transform="translate(${x} ${rowTop + row * lineHeight}) scale(${scale}) translate(6 2)">${body}</g>`
      }).join('')
      const sizeLabels = WORD_WIDTHS.map((w, row) =>
        label(x + 260, rowTop + row * lineHeight + 30, `${w}px wide`, { size: 12, fill: tokens.color['paper-muted'], weight: 'normal' }),
      ).join('')
      return `${label(x, PAD + 22, option.label)}${lines}${sizeLabels}${note(x, rowTop + lineHeight * WORD_WIDTHS.length + 6, option.note)}`
    })
    .join('')

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
  <rect width="${width}" height="${height}" fill="${INK}"/>
  ${label(PAD, height - 20, `${axis.title} — ${axis.question}   ·   240 / 160 / 96 px wide, actual size (96 is the stated minimum)`, { size: 14, fill: tokens.color['paper-muted'], weight: 'normal' })}
  ${columns}
</svg>`
}

const out = join(root, 'tune')
mkdirSync(out, { recursive: true })

for (const axis of AXES) {
  const source = axis.kind === 'mark' ? markSheet(axis) : wordmarkSheet(axis)
  const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const file = join(out, `${axis.id}.png`)
  writeFileSync(file, renderPng(source, width))
  console.log(`${file}  (${axis.options.map((o) => o.id).join(' / ')})`)
}
