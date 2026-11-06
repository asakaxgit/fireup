#!/usr/bin/env node
// Round three. The CEO's note on the tuning sheets was "weight is good, others
// are wrong — use fire", so the lifted-row concept is off the table and this
// pass draws three ways to be fire instead. Kept in the repo as the record.
//
//   node scripts/fire.mjs   ->  brand/fire/*.png
//
// Hard constraint on every option here, from the prior-art check:
//
//   * Firebase's mark is a three-segment faceted flame on a red-orange-yellow
//     gradient. We are a Firebase-compatible emulator, so a flame is the single
//     most confusable shape available to us. No facets, no gradient, and no
//     free-floating symmetrical flame silhouette.
//   * Tinder enforces a trademark over a rounded flame outline and has sent
//     notices over it. Same conclusion: do not draw that silhouette either.
//
// So fire is drawn here out of our own vocabulary — orthogonal rows, the F
// letterform, a flame that is grounded rather than floating — not borrowed.
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
const EMBER_DEEP = tokens.color['ember-deep']
const MUTED = tokens.color['paper-muted']

// --- the three directions ---------------------------------------------------
// Each is a function of (fg, accent) so the same geometry is seen on dark and
// on light without a second drawing.

// A — Kindle. A flame built only from horizontal rows: fire made of records.
// Bars are 6 units on an 8-unit pitch, so every gap equals the bar weight and
// the whole thing stays on the same grid as the wordmark. Widest at the belly,
// tucked in at the base, tip offset right — the asymmetry is what stops a
// stepped taper from reading as a pyramid.
const kindle = (fg, accent) => `
  <rect x="30" y="8"  width="10" height="6" fill="${accent}"/>
  <rect x="24" y="16" width="16" height="6" fill="${accent}"/>
  <rect x="18" y="24" width="24" height="6" fill="${fg}"/>
  <rect x="12" y="32" width="32" height="6" fill="${fg}"/>
  <rect x="10" y="40" width="36" height="6" fill="${fg}"/>
  <rect x="14" y="48" width="30" height="6" fill="${fg}"/>`

// B — Ignite. The F stays, and its middle arm is the flame: a bar off the stem
// whose far end licks upward. The letterform is what survives 16px, so fire
// rides on the part of the drawing that is already legible there.
const ignite = (fg, accent) => `
  <rect x="12" y="9" width="12" height="46" fill="${fg}"/>
  <rect x="12" y="9" width="36" height="12" fill="${fg}"/>
  <path d="M24 28 H40 C50 28 54 21 52 13 C58 27 53 40 38 40 H24 Z" fill="${accent}"/>`

// C — Pilot. A drawn flame sitting on a solid bar. The bar is the point: this
// is fire on your own machine, grounded, not a flame floating in space. Single
// flat colour, flat base, tip leaning — none of which Firebase's or Tinder's
// marks do.
const pilot = (fg, accent) => `
  <rect x="8" y="48" width="48" height="8" fill="${fg}"/>
  <path d="M22 48 C15 38 19 27 32 6 C35 23 45 27 45 38 C45 44 40 48 33 48 Z" fill="${accent}"/>`

// The notes below were rewritten after rendering. Two of the three said
// something different before I looked at the output.
const DIRECTIONS = [
  {
    id: 'kindle',
    label: 'A · Kindle — rejected',
    draw: kindle,
    note:
      'Does not read as fire. It reads as a\n' +
      'stepped pyramid, and at 16px the 6-unit\n' +
      'bars land on 1.5px and smear to grey —\n' +
      'see the raster. Dead at the key size.',
  },
  {
    id: 'ignite',
    label: 'B · Ignite — the direction',
    draw: ignite,
    note:
      'The only one still legible at 16px,\n' +
      'because the F carries it and the flame\n' +
      'rides along. But the tip curls back\n' +
      'into the counter: it reads as a P.\n' +
      'Fixed on the variants sheet.',
  },
  {
    id: 'pilot',
    label: 'C · Pilot — rejected',
    draw: pilot,
    note:
      'It is a water droplet. At every size,\n' +
      'on both backgrounds. That is the worst\n' +
      'available failure for a product called\n' +
      'Fireup, and it is also the silhouette\n' +
      'Firebase and Tinder already own.',
  },
]

// --- round 3b: where the fire goes ------------------------------------------
// B's defect turned out not to be the lick height. Any flame on the *middle*
// arm curls back under the top bar and encloses a bowl, so the mark reads as a
// P at every size — raising or lowering the tip does not fix it, which three
// rendered lick heights showed before this sheet replaced them. The counter of
// an F has to stay open on the right, so the fire has to move off that arm.

// A — Curl. The best of the failed family, kept as the record of what a flame
// on the middle arm does. The bowl it closes is the whole problem.
const curl = (fg, accent) => `
  <rect x="12" y="9" width="12" height="46" fill="${fg}"/>
  <rect x="12" y="9" width="36" height="12" fill="${fg}"/>
  <path d="M24 28 H40 C49 28 52 24 51 19 L54 13 C57 22 54 40 38 40 H24 Z" fill="${accent}"/>`

// B — Torch. The F's top bar is the thing that is alight: it runs out from the
// stem and its far end flares up into a tongue. Nothing encloses anything, so
// the counter stays open and it stays an F.
const torch = (fg, accent) => `
  <rect x="12" y="18" width="12" height="40" fill="${fg}"/>
  <rect x="12" y="34" width="22" height="10" fill="${fg}"/>
  <path d="M12 18 H38 C48 18 52 13 51 5 C58 15 54 28 38 28 H12 Z" fill="${accent}"/>`

// C — Candle. The F is left completely intact and the flame sits on top of the
// stem, the way a flame sits on a wick. Most legible as a letter; the fire is
// an addition rather than a substitution.
const candle = (fg, accent) => `
  <rect x="12" y="24" width="12" height="34" fill="${fg}"/>
  <rect x="12" y="24" width="32" height="10" fill="${fg}"/>
  <rect x="12" y="40" width="21" height="10" fill="${fg}"/>
  <path d="M11 24 C11 14 17 11 18 1 C26 10 26 17 25 24 Z" fill="${accent}"/>`

const VARIANTS = [
  {
    id: 'curl',
    label: 'A · Curl — the failure, kept',
    draw: curl,
    note:
      'A flame on the middle arm closes a bowl\n' +
      'under the top bar. This is a P. Tip\n' +
      'height does not change that.',
  },
  {
    id: 'torch',
    label: 'B · Torch',
    draw: torch,
    note:
      'The top bar is what burns. The counter\n' +
      'stays open on the right, so it stays an\n' +
      'F, and the flame is the one part that\n' +
      'leaves the grid.',
  },
  {
    id: 'candle',
    label: 'C · Candle',
    draw: candle,
    note:
      'F untouched, flame on the wick. The\n' +
      'most legible letter of the three; the\n' +
      'flame is a separate object sitting on\n' +
      'it rather than part of it.',
  },
]

// --- round 3c: the shape of the tongue --------------------------------------
// Torch rendered right, with one complaint: the flare is a horizontal hook, so
// the mark is right-heavy and the tongue reads as a swoosh as much as a flame.
// Three shapes for it, same F underneath.
const stemAndArm = (fg, arm) => `
  <rect x="12" y="16" width="12" height="42" fill="${fg}"/>
  <rect x="12" y="34" width="${arm}" height="12" fill="${fg}"/>`

const TORCHES = [
  {
    id: 'hook',
    label: 'A · Hook — as first drawn',
    arm: 22,
    path: 'M12 16 H36 C47 16 51 11 50 2 C58 13 53 28 36 28 H12 Z',
    note:
      'Flare goes out before it goes up, so\n' +
      'the mark leans right and the tongue\n' +
      'reads as a swoosh.',
  },
  {
    id: 'spire',
    label: 'B · Spire',
    arm: 24,
    path: 'M12 16 H38 C46 16 50 10 49 0 C56 12 52 28 38 28 H12 Z',
    note:
      'Rises before it leans. Longer bar under\n' +
      'it, so the F carries more of the width\n' +
      'and the flame is clearly vertical.',
  },
  {
    id: 'twin',
    label: 'C · Twin tongue',
    arm: 24,
    path: 'M12 16 H32 C39 16 41 11 41 4 C44 10 45 14 46 17 C49 12 52 9 51 0 C58 12 54 28 34 28 H12 Z',
    note:
      'Two licks, the way fire actually has\n' +
      'more than one. Most flame-like at 96px;\n' +
      'the notch is the thing to check small.',
  },
].map(({ id, label, arm, path, note }) => ({
  id,
  label,
  note,
  draw: (fg, accent) => `${stemAndArm(fg, arm)}\n  <path d="${path}" fill="${accent}"/>`,
}))

// --- sheets -----------------------------------------------------------------

const SIZES = [96, 48, 32, 16]
const FONT = 'DejaVu Sans, sans-serif'
const PAD = 28
const COL = 400

const label = (x, y, text, { size = 15, fill = PAPER, weight = 'bold' } = {}) =>
  `<text x="${x}" y="${y}" font-family="${FONT}" font-size="${size}" font-weight="${weight}" fill="${fill}">${text}</text>`

const note = (x, y, text) =>
  text
    .split('\n')
    .map((line, i) => label(x, y + i * 17, line, { size: 13, fill: MUTED, weight: 'normal' }))
    .join('')

// One sheet, all three options side by side, each at every size it is really
// used at plus its actual 16px raster magnified 8x.
const sheet = ({ options, fg, accent, bg, caption }) => {
  const rowTop = PAD + 56
  const zoomTop = rowTop + SIZES[0] + 36
  const zoom = 16 * 8
  const height = zoomTop + zoom + 116
  const width = PAD * 2 + COL * 3

  const columns = options.map((direction, index) => {
    const x = PAD + COL * index
    const body = direction.draw(fg, accent)
    let tileX = x
    const tiles = SIZES.map((size) => {
      const tile = `<g transform="translate(${tileX} ${rowTop + (SIZES[0] - size) / 2}) scale(${size / 64})">${body}</g>`
      tileX += size + 24
      return tile
    }).join('')
    const tiny = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="${bg}"/>${body}</svg>`
    return `
      ${label(x, PAD + 22, direction.label, { fill: fg })}
      ${tiles}
      ${label(x, zoomTop - 12, '16px, magnified 8x — real pixels', { size: 12, fill: MUTED, weight: 'normal' })}
      <g transform="translate(${x} ${zoomTop})">${pixelMap(tiny, 16, 8)}</g>
      ${note(x + zoom + 20, zoomTop + 18, direction.note)}`
  }).join('')

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
  <rect width="${width}" height="${height}" fill="${bg}"/>
  ${label(PAD, height - 24, caption, { size: 14, fill: MUTED, weight: 'normal' })}
  ${columns}
</svg>`
}

const DARK = { fg: PAPER, accent: EMBER, bg: INK }
const LIGHT = { fg: INK, accent: EMBER_DEEP, bg: PAPER }
const SIZE_CAPTION = '96 / 48 / 32 / 16 px, actual size'

const SHEETS = [
  {
    file: 'concepts-dark',
    options: DIRECTIONS,
    ...DARK,
    caption: `Round 3 — use fire   ·   ${SIZE_CAPTION}   ·   on ink, ember #FF5A2B`,
  },
  {
    file: 'concepts-light',
    options: DIRECTIONS,
    ...LIGHT,
    caption: `Round 3 — use fire   ·   ${SIZE_CAPTION}   ·   on paper, ember-deep #C2410C`,
  },
  {
    file: 'variants-dark',
    options: VARIANTS,
    ...DARK,
    caption: `Round 3b — where the fire goes   ·   ${SIZE_CAPTION}   ·   on ink`,
  },
  {
    file: 'variants-light',
    options: VARIANTS,
    ...LIGHT,
    caption: `Round 3b — where the fire goes   ·   ${SIZE_CAPTION}   ·   on paper`,
  },
  {
    file: 'torch-dark',
    options: TORCHES,
    ...DARK,
    caption: `Round 3c — the shape of the tongue   ·   ${SIZE_CAPTION}   ·   on ink`,
  },
  {
    file: 'torch-light',
    options: TORCHES,
    ...LIGHT,
    caption: `Round 3c — the shape of the tongue   ·   ${SIZE_CAPTION}   ·   on paper`,
  },
]

const out = join(root, 'fire')
mkdirSync(out, { recursive: true })

for (const config of SHEETS) {
  const source = sheet(config)
  const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const file = join(out, `${config.file}.png`)
  writeFileSync(file, renderPng(source, width))
  console.log(file)
}

// Each option also standing alone as a real 16px file, for looking at on its
// own rather than next to its siblings.
for (const option of [...DIRECTIONS, ...VARIANTS, ...TORCHES]) {
  const tiny = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="${INK}"/>${option.draw(PAPER, EMBER)}</svg>`
  const file = join(out, `${option.id}-16.png`)
  writeFileSync(file, renderPng(tiny, 16))
  console.log(file)
}
