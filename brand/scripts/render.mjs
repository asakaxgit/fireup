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
