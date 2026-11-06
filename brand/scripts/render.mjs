#!/usr/bin/env node
// Renders an SVG master to PNG at one or more widths.
//
//   node scripts/render.mjs <input.svg> <outDir> <width> [width...]
//
// Used directly for one-off checks; `scripts/export.mjs` is the pipeline that
// regenerates every shipped asset.
import { Resvg } from '@resvg/resvg-js'
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { basename, join } from 'node:path'

export const renderPng = (svg, width) => {
  const resvg = new Resvg(svg, { fitTo: { mode: 'width', value: width } })
  return resvg.render().asPng()
}

// The actual raster at `size` wide, re-expressed as one SVG rect per pixel at
// `scale`. The height follows the source's aspect, so this works on a wide
// lockup as well as on a square mark.
//
// Magnifying a PNG would need an image decoder and a resampler. Drawing the
// decoded pixels back out as unit squares needs neither, and it cannot lie about
// what the renderer produced: every square is one real pixel at its real colour.
export const pixelMap = (svg, size, scale) => {
  const { pixels, width, height } = new Resvg(svg, { fitTo: { mode: 'width', value: size } }).render()
  const hex = (n) => n.toString(16).padStart(2, '0')
  const squares = []
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const at = (y * width + x) * 4
      const [r, g, b, a] = [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
      if (a === 0) continue
      squares.push(
        `<rect x="${x * scale}" y="${y * scale}" width="${scale}" height="${scale}" ` +
          `fill="#${hex(r)}${hex(g)}${hex(b)}"${a === 255 ? '' : ` fill-opacity="${(a / 255).toFixed(3)}"`}/>`,
      )
    }
  }
  return squares.join('')
}

// The tight bounding box of everything a master actually draws, in its own
// viewBox units, measured from the rendered alpha channel.
//
// This used to be three hand-measured constants in build.mjs with a comment
// warning that getting them wrong clips the p. Measuring removes the warning:
// change a stroke weight or let the flame overshoot the cap line, and the
// lockup follows instead of cropping.
export const contentBox = (svg, sample = 1024) => {
  const [minX, minY, vbW] = svg
    .match(/viewBox="([^"]+)"/)[1]
    .trim()
    .split(/[\s,]+/)
    .map(Number)
  const { pixels, width, height } = new Resvg(svg, { fitTo: { mode: 'width', value: sample } }).render()
  let x0 = width
  let y0 = height
  let x1 = -1
  let y1 = -1
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      if (pixels[(y * width + x) * 4 + 3] === 0) continue
      if (x < x0) x0 = x
      if (x > x1) x1 = x
      if (y < y0) y0 = y
      if (y > y1) y1 = y
    }
  }
  if (x1 < 0) throw new Error('contentBox: the master renders nothing')
  const unit = vbW / width
  return {
    x: minX + x0 * unit,
    y: minY + y0 * unit,
    w: (x1 - x0 + 1) * unit,
    h: (y1 - y0 + 1) * unit,
  }
}

export const renderFile = (input, outDir, widths) => {
  const svg = readFileSync(input, 'utf8')
  const stem = basename(input, '.svg')
  mkdirSync(outDir, { recursive: true })
  return widths.map((width) => {
    const out = join(outDir, `${stem}-${width}.png`)
    writeFileSync(out, renderPng(svg, width))
    return out
  })
}

const isMain = import.meta.url === `file://${process.argv[1]}`
if (isMain) {
  const [input, outDir, ...widths] = process.argv.slice(2)
  if (!input || !outDir || widths.length === 0) {
    console.error('usage: render.mjs <input.svg> <outDir> <width> [width...]')
    process.exit(1)
  }
  for (const out of renderFile(input, outDir, widths.map(Number))) console.log(out)
}
