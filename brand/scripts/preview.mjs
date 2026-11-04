#!/usr/bin/env node
// Renders what actually ships, at the sizes it ships at, so it can be looked at
// before it is presented. `npm run build` proves the assets regenerate; this
// proves they are any good.
//
//   node scripts/preview.mjs   ->  brand/preview/*.png
//
// Reads the generated files in site/assets/ rather than the masters, so what is
// on this sheet is the same bytes a browser would load.
import { renderPng, pixelMap } from './render.mjs'
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')
const assets = join(root, '..', 'site', 'assets')
const tokens = JSON.parse(readFileSync(join(root, 'tokens', 'brand.json'), 'utf8'))

const INK = tokens.color.ink
const PAPER = tokens.color.paper
const MUTED = tokens.color['paper-muted']
const FONT = 'DejaVu Sans, sans-serif'
const PAD = 28

const asset = (name) => readFileSync(join(assets, name), 'utf8')
const innerOf = (svg) => svg.replace(/^[\s\S]*?<svg[^>]*>/, '').replace(/<\/svg>\s*$/, '')
const viewBoxOf = (svg) => svg.match(/viewBox="([^"]+)"/)[1]

const label = (x, y, text, { size = 14, fill = PAPER, weight = 'bold' } = {}) =>
  `<text x="${x}" y="${y}" font-family="${FONT}" font-size="${size}" font-weight="${weight}" fill="${fill}">${text}</text>`
const caption = (x, y, text) => label(x, y, text, { size: 12, fill: MUTED, weight: 'normal' })

// Places a generated SVG at an exact height, preserving its own aspect ratio.
const place = (svg, x, y, height) => {
  const [, , vw, vh] = viewBoxOf(svg).split(/[\s,]+/).map(Number)
  const width = (vw / vh) * height
  return {
    width,
    svg: `<svg viewBox="${viewBoxOf(svg)}" x="${x}" y="${y}" width="${width.toFixed(2)}" height="${height}">${innerOf(svg)}</svg>`,
  }
}

const sheets = []

// --- 1. the mark and the lockup, on both grounds, at real sizes -------------
const MARK_SIZES = [96, 64, 48, 32, 24, 16]
const LOCKUP_WIDTHS = [320, 200, 96]

const identitySheet = (mode) => {
  const bg = mode === 'dark' ? INK : PAPER
  const fg = mode === 'dark' ? PAPER : INK
  const mark = asset(`fireup-mark-${mode}.svg`)
  const lock = asset(`fireup-lockup-${mode}.svg`)

  let x = PAD
  const marks = MARK_SIZES.map((size) => {
    const { width, svg } = place(mark, x, PAD + 30 + (MARK_SIZES[0] - size), size)
    const tile = svg + caption(x, PAD + 30 + MARK_SIZES[0] + 18, `${size}`)
    x += width + 28
    return tile
  }).join('')

  let y = PAD + 30 + MARK_SIZES[0] + 56
  const lockups = LOCKUP_WIDTHS.map((w) => {
    const [, , vw, vh] = viewBoxOf(lock).split(/[\s,]+/).map(Number)
    const height = (w * vh) / vw
    const tile =
      `<svg viewBox="${viewBoxOf(lock)}" x="${PAD}" y="${y}" width="${w}" height="${height.toFixed(2)}">${innerOf(lock)}</svg>` +
      caption(PAD + 340, y + height / 2, `${w}px wide${w === 96 ? '  — the stated minimum' : ''}`)
    y += height + 26
    return tile
  }).join('')

  const width = PAD * 2 + 660
  const height = y + PAD + 10
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
  <rect width="${width}" height="${height}" fill="${bg}"/>
  ${label(PAD, PAD + 6, `Mark and lockup on ${mode === 'dark' ? 'ink' : 'paper'} — actual size`, { fill: fg })}
  ${marks}
  ${lockups}
</svg>`
}

sheets.push(['identity-dark', identitySheet('dark')], ['identity-light', identitySheet('light')])

// --- 2. the favicons, at 1:1 and as the real pixels that land ---------------
// Each favicon is its own drawing rather than the mark scaled down, so this is
// the only place the three drawings can be compared against each other.
const FAVICONS = [
  ['favicon-16', 16, 'drawn for 16 — one tongue, muzzle on one pixel row'],
  ['favicon-32', 32, 'drawn for 32 — four steps, so the fire tapers'],
]

const faviconSheet = () => {
  const zoom = 10
  let x = PAD
  const columns = FAVICONS.map(([name, size, note]) => {
    const svg = readFileSync(join(root, 'logo', `${name}.svg`), 'utf8')
    const block =
      label(x, PAD + 6, `${name}.svg`) +
      `<svg viewBox="0 0 ${size} ${size}" x="${x}" y="${PAD + 22}" width="${size}" height="${size}"/>` +
      `<g transform="translate(${x} ${PAD + 22})">${pixelMap(svg, size, 1)}</g>` +
      caption(x + size + 14, PAD + 22 + size / 2, '1:1') +
      caption(x, PAD + 22 + 48, `${size}px, magnified ${zoom}x — real pixels`) +
      `<g transform="translate(${x} ${PAD + 22 + 56})">${pixelMap(svg, size, zoom)}</g>` +
      caption(x, PAD + 22 + 56 + size * zoom + 22, note)
    x += Math.max(size * zoom, 200) + 48
    return block
  }).join('')

  const width = x + PAD - 48
  const height = PAD + 22 + 56 + 32 * zoom + 48
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
  <rect width="${width}" height="${height}" fill="${INK}"/>
  ${columns}
</svg>`
}

sheets.push(['favicons', faviconSheet()])

// --- 3. the app icon, at the size the last two defects only showed up at ----
// Round 3's flame read as a horn and round 4's outer lobe was clipped flat by
// the viewBox; neither was visible below 512px. The shipped PNG is decoded back
// into squares rather than re-rendered from the master, so this cannot flatter
// the pipeline.
const appIconSheet = () => {
  const size = 512
  const svg = readFileSync(join(root, 'logo', 'mark.svg'), 'utf8')
  const field = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="${INK}"/>${innerOf(svg)}</svg>`
  const width = PAD * 2 + size + 320
  const height = PAD * 2 + size + 34
  // 1:1, not scaled down to fit the sheet: at 0.5 the unit squares land on half
  // pixels, resvg antialiases every seam, and the whole icon goes grey. A
  // preview that misreports the colour is worse than no preview.
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">
  <rect width="${width}" height="${height}" fill="${INK}"/>
  ${label(PAD, PAD + 6, 'The 64-unit mark on an ink field, at 512 — real pixels, 1:1')}
  <g transform="translate(${PAD} ${PAD + 24})">${pixelMap(field, size, 1)}</g>
  ${caption(PAD + size + 28, PAD + 60, 'The two defects that got through review —')}
  ${caption(PAD + size + 28, PAD + 78, 'a flame that read as a horn, and an outer')}
  ${caption(PAD + size + 28, PAD + 96, 'lobe clipped flat by the viewBox — were')}
  ${caption(PAD + size + 28, PAD + 114, 'both invisible below this size.')}
</svg>`
}

sheets.push(['app-icon', appIconSheet()])

const out = join(root, 'preview')
mkdirSync(out, { recursive: true })

for (const [name, source] of sheets) {
  const width = Number(source.match(/viewBox="0 0 ([\d.]+)/)[1])
  const file = join(out, `${name}.png`)
  writeFileSync(file, renderPng(source, Math.round(width)))
  console.log(file)
}
