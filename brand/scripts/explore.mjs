#!/usr/bin/env node
// Renders the three identity directions at the sizes they would be used at,
// on both dark and light, so they can be compared before one is polished.
// Kept in the repo as the record of what was explored and why.
//
//   node scripts/explore.mjs   ->  brand/explore/*.png
import { renderPng } from './render.mjs'
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')
const tokens = JSON.parse(readFileSync(join(root, 'tokens', 'brand.json'), 'utf8'))

// Each direction is a function of (fg, accent) so the same geometry can be seen
// on dark and on light without a second drawing.
const DIRECTIONS = {
  // A — Ascend: table rows, the top one lifted clear of the stack.
  // The product's job in one glyph: rows come out of a document store.
  ascend: (fg, accent) => `
    <rect x="6"  y="42" width="52" height="8" rx="2" fill="${fg}"/>
    <rect x="6"  y="28" width="38" height="8" rx="2" fill="${fg}"/>
    <rect x="20" y="14" width="38" height="8" rx="2" fill="${accent}"/>`,

  // B — Ignite F: a slab monogram, middle arm in the accent.
  ignite: (fg, accent) => `
    <rect x="16" y="11" width="12" height="44" fill="${fg}"/>
    <rect x="16" y="11" width="34" height="12" fill="${fg}"/>
    <rect x="16" y="29" width="24" height="12" fill="${accent}"/>`,

  // C — Portal: a local instance, with something breaking out of the top.
  // Rejected after rendering: this is the IEC 5009 power symbol. It reads as a
  // button, not a brand, and a universal standard glyph is not ownable.
  portal: (fg, accent) => `
    <path d="M38 8 H44 A12 12 0 0 1 56 20 V44 A12 12 0 0 1 44 56 H20 A12 12 0 0 1 8 44 V20 A12 12 0 0 1 20 8 H26"
          fill="none" stroke="${fg}" stroke-width="8"/>
    <rect x="28" y="0" width="8" height="30" fill="${accent}"/>`,

  // Round two. A read as a hamburger menu and C was unusable, so the second
  // pass crosses A's idea (rows in motion) into B's monogram.

  // B2 — Liftoff: the F's middle arm has come off the stem and moved right.
  // The counter under it is the gap a row leaves when it is taken out.
  liftoff: (fg, accent) => `
    <rect x="16" y="11" width="12" height="44" fill="${fg}"/>
    <rect x="16" y="11" width="34" height="12" fill="${fg}"/>
    <rect x="32" y="29" width="22" height="12" fill="${accent}"/>`,

  // A2 — Stagger: the rows again, but right-aligned so the left edge makes a
  // diagonal. No longer a menu icon; the stack is clearly going somewhere.
  stagger: (fg, accent) => `
    <rect x="6"  y="42" width="52" height="8" rx="2" fill="${fg}"/>
    <rect x="20" y="28" width="38" height="8" rx="2" fill="${fg}"/>
    <rect x="34" y="14" width="24" height="8" rx="2" fill="${accent}"/>`,
}

const svg = (body) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">${body}\n</svg>`

const SIZES = [512, 128, 48, 32, 16]
const MODES = {
  dark: { fg: tokens.color.paper, accent: tokens.color.ember, bg: tokens.color.ink },
  light: { fg: tokens.color.ink, accent: tokens.color['ember-deep'], bg: tokens.color.paper },
}

const out = join(root, 'explore')
mkdirSync(out, { recursive: true })

// One contact sheet per direction per mode: every size on one strip, each sized
// as it would really appear, so 16px is judged at 16px.
const sheet = (name, mode) => {
  const { fg, accent, bg } = MODES[mode]
  const body = DIRECTIONS[name](fg, accent)
  const pad = 24
  const gap = 32
  const width = SIZES.reduce((sum, s) => sum + s + gap, pad * 2 - gap)
  const height = SIZES[0] + pad * 2
  let x = pad
  const tiles = SIZES.map((size) => {
    const tile = `<g transform="translate(${x} ${pad + (SIZES[0] - size) / 2}) scale(${size / 64})">${body}</g>`
    x += size + gap
    return tile
  }).join('\n')
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
    <rect width="${width}" height="${height}" fill="${bg}"/>
    ${tiles}
  </svg>`
}

for (const name of Object.keys(DIRECTIONS)) {
  for (const mode of Object.keys(MODES)) {
    const file = join(out, `${name}-${mode}.png`)
    const source = sheet(name, mode)
    const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
    writeFileSync(file, renderPng(source, width))
    console.log(file)
  }
  // And the honest test: each direction standing alone at 16px, scaled up 8x
  // afterwards only so a human can see which pixels actually landed.
  const { fg, accent, bg } = MODES.dark
  const tiny = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="${bg}"/>${DIRECTIONS[name](fg, accent)}</svg>`
  writeFileSync(join(out, `${name}-16.png`), renderPng(tiny, 16))
}
