#!/usr/bin/env node
// Round five. The CEO's note on round 4: "4a is almost good. Omit nozzle
// entirely." So the half-weight muzzle comes out of the drawing and does not
// come back. What is left is the F and the fire blowing up and to the right,
// which is the rest of the round-4 note unchanged.
//
//   node scripts/plain.mjs   ->  brand/plain/*.png
//
// Taking the muzzle out reopens the one question it was answering. The fire
// used to sit on a 6-unit nozzle, and its 9-unit base was *wider* than that
// nozzle, so the join read as the fire swallowing the tip. Against the bar's
// full 12 weight the same base is narrower than what it sits on, and the join
// becomes a step. Two sheets:
//   5a  where the fire meets the squared-off end of the bar
//   5b  how far the bar runs before it does, with 5a's answer applied
//
// Constraints carried forward, unchanged:
//   * No faceted, gradient, free-floating flame silhouette — that is Firebase's
//     mark, and we are a Firebase-compatible emulator. Tinder separately
//     enforces a rounded flame outline.
//   * The F's counter stays open on the right or it stops being an F at 16px.
//   * Nothing lands outside x<=60 / y>=1: round 4's first drawing ran past the
//     64-unit viewBox and the renderer cut the outer lobe flat.
import { renderPng } from './render.mjs'
import { sheet } from './sheet.mjs'
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

// The F, on a 64-unit grid, with no muzzle. `barEnd` is where the top bar
// stops; everything else is the letter as approved in round 4.
//   stem    x6..18      y16..60   (weight 12)
//   top bar x6..barEnd  y16..28
//   middle  x6..28      y34..46
const letter = (fg, barEnd) => `
  <rect x="6" y="16" width="12" height="44" fill="${fg}"/>
  <rect x="6" y="16" width="${barEnd - 6}" height="12" fill="${fg}"/>
  <rect x="6" y="34" width="22" height="12" fill="${fg}"/>`

// The three jets are written with x relative to the bar's end and y absolute,
// so each is placed by translating in x alone. All three keep round 4b-C's
// silhouette — a belly, a notch, then a second shorter tongue, the notch being
// what makes it read as fire rather than as a swoosh — and all three pin the
// tip at y=1 and the outer tongue at x+20, so the mark's height and width do
// not move between options. Only the base moves.
const JETS = {
  // Base spans the bar's full 12 weight. The bottom controls sit at exactly 28
  // rather than 29: a unit of bulge below the bar reads as a drip at 512px.
  terminal:
    'M0 16 C5 14 8 11 10 7 C12 4 13 3 14 1 C15 7 15 12 14 16 ' +
    'C17 14 19 12 20 10 C19 17 16 24 11 27 C7 28 3 28 0 28 Z',
  // Round 4's base, 9 units, unchanged — this is the shipped path shifted.
  taper:
    'M0 18 C5 16 8 13 10 9 C12 5 13 3 14 1 C15 7 15 12 14 16 ' +
    'C17 14 19 12 20 10 C19 17 16 23 11 26 C7 28 3 28 0 27 Z',
  // Base on the bar's top half only; the bar's end stays square beneath it.
  lift:
    'M0 16 C5 14 8 11 10 7 C12 4 13 3 14 1 C15 7 15 12 14 16 ' +
    'C17 14 19 12 20 10 C19 16 16 21 11 23 C7 24 3 24 0 23 Z',
}

const draw = (barEnd, jet) => (fg, accent) =>
  `${letter(fg, barEnd)}\n  <g transform="translate(${barEnd} 0)"><path d="${JETS[jet]}" fill="${accent}"/></g>`

// --- 5a: where the fire meets the end of the bar -----------------------------
// Bar end held at 36 for all three so the only variable is the join.
const JOIN_BAR = 36

const JOINS = [
  {
    id: 'terminal',
    label: 'A · Terminal — the bar end is the fire',
    draw: draw(JOIN_BAR, 'terminal'),
    note:
      "The jet base spans the bar's full 12\n" +
      'weight, so there is no step at all — the\n' +
      'bar runs out and catches.',
  },
  {
    id: 'taper',
    label: "B · Taper — round 4's base, unchanged",
    draw: draw(JOIN_BAR, 'taper'),
    note:
      'The 9-unit base that sat on the muzzle,\n' +
      'now against the full bar. Notched 2 units\n' +
      'above the bar and 1 below.',
  },
  {
    id: 'lift',
    label: 'C · Lift — off the top-right corner',
    draw: draw(JOIN_BAR, 'lift'),
    note:
      'The fire leaves the top half only and the\n' +
      'bar stays squared off under it. The most\n' +
      'literal reading of "from right top side".',
  },
]

// --- 5b: how far the bar runs ------------------------------------------------
// JOIN is set from 5a after looking at it, not before.
const JOIN = 'terminal'

const LENGTHS = [
  {
    id: 'short',
    label: 'A · Bar to 32 — more fire',
    draw: draw(32, JOIN),
    note:
      'The top bar ends only 4 units past the\n' +
      'middle arm, and the fire is the widest\n' +
      'thing in the mark.',
  },
  {
    id: 'mid',
    label: 'B · Bar to 36',
    draw: draw(36, JOIN),
    note: 'A 30-unit bar against a 22-unit middle arm.',
  },
  {
    id: 'long',
    label: 'C · Bar to 40 — more letter',
    draw: draw(40, JOIN),
    note:
      'The outer tongue lands on x=60, which is\n' +
      "where round 4's silhouette ended. Any\n" +
      'longer and the viewBox clips it.',
  },
]

// --- sheets ------------------------------------------------------------------

const COL = 470
const DARK = { fg: PAPER, accent: EMBER, bg: INK, muted: MUTED, col: COL }
const LIGHT = { fg: INK, accent: EMBER_DEEP, bg: PAPER, muted: '#5A6782', col: COL }
const SIZE_CAPTION = '96 / 48 / 32 / 16 px, actual size'

const SHEETS = [
  {
    file: 'joins-dark',
    options: JOINS,
    ...DARK,
    caption: `Round 5a — no muzzle: where the fire meets the bar   ·   ${SIZE_CAPTION}   ·   on ink, ember #FF5A2B`,
  },
  {
    file: 'joins-light',
    options: JOINS,
    ...LIGHT,
    caption: `Round 5a — no muzzle: where the fire meets the bar   ·   ${SIZE_CAPTION}   ·   on paper, ember-deep #C2410C`,
  },
  {
    file: 'lengths-dark',
    options: LENGTHS,
    ...DARK,
    caption: `Round 5b — how far the bar runs   ·   ${SIZE_CAPTION}   ·   on ink`,
  },
  {
    file: 'lengths-light',
    options: LENGTHS,
    ...LIGHT,
    caption: `Round 5b — how far the bar runs   ·   ${SIZE_CAPTION}   ·   on paper`,
  },
]

const out = join(root, 'plain')
mkdirSync(out, { recursive: true })

for (const config of SHEETS) {
  const source = sheet(config)
  const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const file = join(out, `${config.file}.png`)
  writeFileSync(file, renderPng(source, width))
  console.log(file)
}

// Each option standing alone at the two sizes that decide it: 16, where the
// letter has to survive, and 512, where round 4's clipped lobe hid.
for (const option of [...JOINS, ...LENGTHS]) {
  const tiny = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="${INK}"/>${option.draw(PAPER, EMBER)}</svg>`
  for (const size of [16, 512]) {
    const file = join(out, `${option.id}-${size}.png`)
    writeFileSync(file, renderPng(tiny, size))
    console.log(file)
  }
}
