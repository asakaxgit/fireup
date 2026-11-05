#!/usr/bin/env node
// Regenerates every shipped brand asset from the masters in brand/logo/ and
// brand/tokens/brand.json. One command, no hand-edited output:
//
//   cd brand && npm run build
//
// Writes:
//   brand/tokens/brand.css   CSS custom properties, for the site and anyone else
//   site/assets/*.svg        colour variants and lockups, composed from masters
//   site/assets/*.png        favicons, app icons, social card
import { renderPng, contentBox } from './render.mjs'
import { master, innerOf, recolour, spell, W, DOT } from './word.mjs'
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const brand = join(here, '..')
const assets = join(brand, '..', 'site', 'assets')
const tokens = JSON.parse(readFileSync(join(brand, 'tokens', 'brand.json'), 'utf8'))
const c = tokens.color

const write = (name, text) => {
  writeFileSync(join(assets, name), text)
  return name
}

mkdirSync(assets, { recursive: true })
const written = []

// --- tokens -----------------------------------------------------------------
// brand.json is the source; the CSS is a projection of it so the two cannot drift.
const cssBlock = (group, prefix) =>
  Object.entries(tokens[group])
    .map(([key, value]) => `  --fu-${prefix}-${key}: ${value};`)
    .join('\n')

const css = `/* Fireup design tokens — GENERATED from brand/tokens/brand.json by
 * brand/scripts/build.mjs. Do not edit by hand; edit the JSON and rebuild.
 *
 * ${tokens.status}
 */
:root {
${cssBlock('color', 'color')}

${cssBlock('font', 'font')}

${cssBlock('size', 'size')}

${cssBlock('space', 'space')}

${cssBlock('radius', 'radius')}
}
`
writeFileSync(join(brand, 'tokens', 'brand.css'), css)
// The site consumes the same file rather than a second copy of the values.
written.push(write('tokens.css', css))

// --- colour variants of the two masters -------------------------------------
// Substitution, not re-drawing, so the geometry has exactly one definition.
const VARIANTS = {
  dark: [c.paper, c.ember], // for ink backgrounds
  light: [c.ink, c['ember-deep']], // for white backgrounds
  mono: ['currentColor', 'currentColor'], // takes the surrounding text colour
}

for (const [variant, [fg, accent]] of Object.entries(VARIANTS)) {
  written.push(write(`fireup-mark-${variant}.svg`, recolour(master('mark'), fg, accent)))
  written.push(write(`fireup-wordmark-${variant}.svg`, recolour(master('wordmark'), fg, accent)))
}

// --- lockup -----------------------------------------------------------------
// The mark is not placed beside the word. It *is* the word's F: the lockup is
// "Fireup" with the approved mark as the initial letter and the wordmark's own
// monoline f dropped, because that letter is now drawn twice otherwise.
// Decided in round 6 — brand/scripts/mix.mjs holds the three directions that
// were drawn and the two that were rejected.
//
// Composed from the masters, never re-drawn, so a change to either reaches the
// lockup. The mark is read whole and transformed; the word is read as its own
// paths with the first two (the f) left out.
const MARK_BOX = contentBox(master('mark'))

// The word's construction numbers — baseline, the i's tittle, where "ireup"
// starts, and the letter gap — come from the wordmark master itself via
// word.mjs. They used to be retyped here, which is how round 6 shipped a
// letter gap of 14 against a drawing that uses 8.
const BASELINE = W.baseline
const IREUP_LEFT = W.ireupL
const LETTER_GAP = W.letterGap

const capLine = (name) => {
  const svg = master(name)
  const read = (attr) => Number(svg.match(new RegExp(`data-${attr}="([-\\d.]+)"`))[1])
  return { top: read('cap-top'), bottom: read('cap-bottom') }
}
const MARK_CAP = capLine('mark')
const MARK_FOOT = MARK_CAP.bottom // the F stands on the word's baseline
const FIRE_FOOT = 28 // the underside of the bar, and of the fire sitting on it
const ARM_END = 28 // how far the F's middle arm runs
const STEM_LEFT = 6

// How big the F is. The whole bar and fire overhang the lowercase, and the one
// thing under them that reaches up into that band is the i's tittle, so the
// mark's size and the dot's height are the same question:
//
//   bar underside = BASELINE - (MARK_FOOT - FIRE_FOOT) * SCALE
//   tittle top    = DOT.cy - DOT.r
//
// Below a 0 clearance the dot lands inside the fire and the two merge into one
// blob by 128px. 3 units — a quarter of a bar — is the first value where the
// dot is still its own pixel at the 96px minimum, checked on the magnified
// raster in brand/mix/zoom-size.png rather than by eye.
//
// Round 6 solved that equation for SCALE and got 1.5, which is the size the
// lockup was signed off at. Round 11 brought the tittle down 4 units, and
// re-solving would have returned 1.375 — an 8% smaller mark, a 2.01:1 lockup
// gone to 2.14:1, and the site header, the clear-space unit and the social card
// all moving with it, none of which anybody asked for. Rendered at the 96px
// minimum the smaller mark is also simply worse: the F's counter muddies and
// the bar loses its edge (brand/align/zoom-size.png, 11b).
//
// So the 3 units are a FLOOR the mark has to clear, not a size to re-derive
// from every time a letter moves. SCALE is frozen at the approved value and the
// equation is asserted instead of solved — which still catches the thing it was
// written to catch, a drawing change that puts the dot into the fire, and no
// longer resizes an approved mark as a side effect of moving a dot.
const CLEARANCE = 3
const SCALE = 1.5 // round 6, signed off. See above before changing it.
const DOT_AIR = DOT.cy - DOT.r - (BASELINE - (MARK_FOOT - FIRE_FOOT) * SCALE)
if (DOT_AIR < CLEARANCE) {
  throw new Error(
    `the mark's bar clears the i's tittle by ${DOT_AIR.toFixed(2)} units, under the ${CLEARANCE} it needs. ` +
      `Either raise the dot (logo/wordmark.svg) or take SCALE down to ${((BASELINE - (DOT.cy - DOT.r) + CLEARANCE) / (MARK_FOOT - FIRE_FOOT)).toFixed(4)} — ` +
      'and read what that second one costs in the note above first.',
  )
}

const markX = (u) => (u - STEM_LEFT) * SCALE // mark units -> lockup units, stem on x=0
const markY = (v) => BASELINE + (v - MARK_FOOT) * SCALE
const WORD_X = markX(ARM_END) + LETTER_GAP // where "ireup" starts
const SHIFT = WORD_X - IREUP_LEFT

// What the lockup spells, named rather than counted. The F is absent because
// the mark is that letter. The arrow is part of the word, not of the mark — it
// is the accent over the U — so it travels with the letters and shifts with
// them.
const IREUP = ['i', 'r', 'e', 'U', 'p', 'tittle', 'arrow']

const lockupBody = (fg, accent) => `<g transform="translate(${(-STEM_LEFT * SCALE).toFixed(2)} ${(BASELINE - MARK_FOOT * SCALE).toFixed(2)}) scale(${SCALE.toFixed(4)})">${innerOf(recolour(master('mark'), fg, accent))}</g>
  ${spell(IREUP, { shift: SHIFT, fg, accent })}`

// The viewBox is the rendered alpha's own box, so the flame's overshoot above
// the cap and the p's descender below the baseline both set it. A hand-counted
// box is what clipped the flame in round 4.
const LOCK_BOX = contentBox(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="-80 -100 480 240">${lockupBody('#FFFFFF', '#FF5A2B')}</svg>`)
const TOP = LOCK_BOX.y
const HEIGHT = LOCK_BOX.h
const LOCKUP_W = LOCK_BOX.w
const CLEAR = 12 * SCALE // one bar of the mark — the clear-space unit

const lockup = (fg, accent) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${LOCK_BOX.x.toFixed(2)} ${TOP.toFixed(2)} ${LOCKUP_W.toFixed(2)} ${HEIGHT.toFixed(2)}" role="img" aria-label="FireUp">
  <!-- GENERATED by brand/scripts/build.mjs from logo/mark.svg + logo/wordmark.svg.
       The mark is the word's F: it stands on the word's baseline at ${SCALE.toFixed(2)}x, and
       its bar and fire overhang the lowercase, clearing the i's tittle by ${DOT_AIR.toFixed(1)}
       against a floor of ${CLEARANCE}.
       The word's own F is not drawn — the mark is that letter. The arrow over
       the U comes from the wordmark and travels with the letters.
       Clear space on all sides: ${CLEAR.toFixed(1)} units of this viewBox.
       Minimum size: ${tokens.logo.minSizeLockup} -->
  ${lockupBody(fg, accent)}
</svg>
`

for (const [variant, [fg, accent]] of Object.entries(VARIANTS)) {
  written.push(write(`fireup-lockup-${variant}.svg`, lockup(fg, accent)))
}

// --- rasters ----------------------------------------------------------------
// Favicons come from the size-specific masters, never from the 64-unit mark.
const raster = (name, svg, width) => {
  writeFileSync(join(assets, name), renderPng(svg, width))
  written.push(name)
}

raster('favicon-16.png', master('favicon-16'), 16)
raster('favicon-32.png', master('favicon-32'), 32)
raster('favicon-48.png', master('favicon-32'), 48) // 32 master upscales cleanly at 48 (1.5x of 2x grid)
written.push(write('favicon.svg', master('favicon-32')))

// App icons: the 64-unit mark on an ink field, with the padding each platform
// expects. iOS applies its own mask, so the field is square and full bleed.
const appIcon = (size, inset) => {
  const inner = innerOf(master('mark'))
  const box = size - inset * 2
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${size} ${size}">
  <rect width="${size}" height="${size}" fill="${c.ink}"/>
  <svg viewBox="${MARK_BOX.x.toFixed(2)} ${MARK_BOX.y.toFixed(2)} ${MARK_BOX.w.toFixed(2)} ${MARK_BOX.h.toFixed(2)}" x="${(inset + (box - (box * MARK_BOX.w) / MARK_BOX.h) / 2).toFixed(2)}" y="${inset}" width="${((box * MARK_BOX.w) / MARK_BOX.h).toFixed(2)}" height="${box}">${inner}</svg>
</svg>`
}

raster('apple-touch-icon-180.png', appIcon(180, 32), 180)
raster('icon-192.png', appIcon(192, 36), 192)
raster('icon-512.png', appIcon(512, 96), 512)
// Maskable icons are cropped to a circle of 80% diameter, so the safe area is smaller.
raster('icon-maskable-512.png', appIcon(512, 134), 512)

// Social card: lockup on ink, no body copy — there is no licensed display font
// to set it in, and a drawn wordmark says the name better than DejaVu would.
const og = () => {
  const w = 1200
  const h = 630
  const lock = innerOf(lockup(c.paper, c.ember))
  const lockWidth = 560
  const lockHeight = (lockWidth * HEIGHT) / LOCKUP_W
  // The rule hangs off the lockup's measured bottom, and the pair is centred as
  // one block. Pinning the rule to a fraction of the card is what put it
  // through the middle of the word when the lockup went from 3:1 to 2:1.
  const ruleGap = 60
  const ruleH = 4
  const top = (h - (lockHeight + ruleGap + ruleH)) / 2
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${h}">
  <rect width="${w}" height="${h}" fill="${c.ink}"/>
  <rect x="0" y="0" width="${w}" height="6" fill="${c.ember}"/>
  <svg viewBox="${LOCK_BOX.x.toFixed(2)} ${TOP.toFixed(2)} ${LOCKUP_W.toFixed(2)} ${HEIGHT.toFixed(2)}" x="${(w - lockWidth) / 2}" y="${top.toFixed(2)}" width="${lockWidth}" height="${lockHeight.toFixed(2)}">${lock}</svg>
  <rect x="${(w - 240) / 2}" y="${(top + lockHeight + ruleGap).toFixed(2)}" width="240" height="${ruleH}" fill="${c['rule-dark']}"/>
</svg>`
}

raster('og-card.png', og(), 1200)

console.log(written.sort().join('\n'))
console.log(`\n${written.length} assets written to site/assets/, plus brand/tokens/brand.css`)
