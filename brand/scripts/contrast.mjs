#!/usr/bin/env node
// Verifies every palette pair the brand actually uses against WCAG 2.1 AA.
// Run with `npm run contrast` in brand/. Exits non-zero if a required pair fails.
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const tokens = JSON.parse(readFileSync(join(here, '..', 'tokens', 'brand.json'), 'utf8'))

const channel = (v) => {
  const s = v / 255
  return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4
}

export const luminance = (hex) => {
  const n = parseInt(hex.replace('#', ''), 16)
  const r = channel((n >> 16) & 0xff)
  const g = channel((n >> 8) & 0xff)
  const b = channel(n & 0xff)
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

export const ratio = (a, b) => {
  const la = luminance(a)
  const lb = luminance(b)
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05)
}

const color = (name) => {
  const hex = tokens.color[name]
  if (!hex) throw new Error(`no such colour token: ${name}`)
  return hex
}

// `min` is the AA threshold for how the pair is used:
// 4.5 body text, 3.0 large text (>=24px, or >=18.66px bold) and UI boundaries.
const PAIRS = [
  ['paper', 'ink', 4.5, 'body text on light'],
  ['paper', 'ink-muted', 4.5, 'secondary text on light'],
  ['paper', 'ember-deep', 4.5, 'accent text / links on light'],
  ['paper', 'azure-deep', 4.5, 'secondary accent text on light'],
  ['paper-sunk', 'ink', 4.5, 'body text on sunk panels'],
  ['paper-sunk', 'ink-muted', 4.5, 'secondary text on sunk panels'],
  ['ink', 'paper', 4.5, 'body text on dark'],
  ['ink', 'paper-muted', 4.5, 'secondary text on dark'],
  ['ink', 'ember', 4.5, 'accent text on dark'],
  ['ink', 'azure', 4.5, 'secondary accent text on dark'],
  ['ink-raised', 'paper', 4.5, 'body text on raised dark panels'],
  ['ink-raised', 'paper-muted', 4.5, 'secondary text on raised dark panels'],
  ['ink-raised', 'ember', 4.5, 'accent text on raised dark panels'],
  ['ember-deep', 'paper', 4.5, 'text on the primary button'],
  ['ink', 'border-dark', 3.0, 'meaningful edges on dark'],
  ['paper', 'border-light', 3.0, 'meaningful edges on light'],
  ['ink', 'ember', 3.0, 'logo mark on dark'],
  ['paper', 'ember-deep', 3.0, 'logo mark on light'],
  ['paper', 'ok-deep', 4.5, 'status: supported, on light'],
  ['paper', 'warn-deep', 4.5, 'status: partial, on light'],
  ['ink', 'ok', 4.5, 'status: supported, on dark'],
  ['ink', 'warn', 4.5, 'status: partial, on dark'],
]

const rows = PAIRS.map(([bg, fg, min, use]) => {
  const r = ratio(color(bg), color(fg))
  return { bg, fg, min, use, r, pass: r >= min }
})

const w = (s, n) => String(s).padEnd(n)
console.log(`${w('background', 12)} ${w('foreground', 12)} ${w('ratio', 8)} ${w('min', 5)} result  use`)
for (const row of rows) {
  console.log(
    `${w(row.bg, 12)} ${w(row.fg, 12)} ${w(`${row.r.toFixed(2)}:1`, 8)} ${w(row.min.toFixed(1), 5)} ` +
      `${row.pass ? 'PASS  ' : 'FAIL  '}  ${row.use}`,
  )
}

const failed = rows.filter((row) => !row.pass)
console.log(`\n${rows.length - failed.length}/${rows.length} pairs meet WCAG AA for their use`)
process.exit(failed.length ? 1 : 0)
