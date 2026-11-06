#!/usr/bin/env node
// Round four. The CEO's note: "treat F as a lighter gun and blow fire from
// right top side." So the F is read as a trigger lighter — stem as the grip,
// middle arm as the trigger, top bar as the nozzle — and the fire leaves the
// muzzle to the right instead of rising off the bar.
//
//   node scripts/lighter.mjs   ->  brand/lighter/*.png
//
// Four sheets, each one answering the defect the one before it rendered:
//   4a  how literally the lighter is drawn (letter-first .. appliance-first)
//   4b  the shape of the jet, after 4a's fire hooked back over the bar
//   4c  how the fire leaves the muzzle, after 4b's flame hid the muzzle
//   4d  which lighter cues are coarse enough to survive, after 4c found none
//
// Every note below was written once before rendering and rewritten after. The
// pre-render version was wrong on 4a-C, 4b-B, 4c and 4d-C.
//
// Constraints carried forward, unchanged:
//   * No faceted, gradient, free-floating flame silhouette — that is Firebase's
//     mark, and we are a Firebase-compatible emulator. Tinder separately
//     enforces a rounded flame outline.
//   * The F's counter stays open on the right or it stops being an F at 16px.
//   * Drawn as a utility trigger lighter, not a pistol: no bore, no sight, no
//     hammer, no magazine, no grip angle. A mark that reads as a firearm gets
//     rejected by app stores and ad review regardless of what we meant by it.
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

// --- 4a: how literal ---------------------------------------------------------

// A — Nozzle. The letter is untouched in proportion; the only concession to the
// lighter is that the top bar's end steps in to a narrower muzzle. At 16px that
// step is half a pixel and disappears, which is the point: small, it is a plain
// F with fire coming off it.
const nozzle = (fg, accent) => `
  <rect x="12" y="16" width="12" height="44" fill="${fg}"/>
  <rect x="12" y="16" width="24" height="12" fill="${fg}"/>
  <rect x="36" y="17" width="8"  height="10" fill="${fg}"/>
  <rect x="12" y="34" width="22" height="12" fill="${fg}"/>
  <path d="M43 17 C50 16 55 12 58 3 C63 11 63 22 55 26 C50 29 46 28 43 27 Z" fill="${accent}"/>`

// B — Trigger. The middle arm is cut as a trigger — a wedge, not a bar — and
// the grip flares at the foot. Reads as the object first, the letter second.
const trigger = (fg, accent) => `
  <path d="M12 16 H24 V52 L27 60 H9 L12 52 Z" fill="${fg}"/>
  <rect x="12" y="16" width="22" height="12" fill="${fg}"/>
  <rect x="34" y="18" width="12" height="8" fill="${fg}"/>
  <path d="M12 34 H28 L33 40 L28 46 H12 Z" fill="${fg}"/>
  <path d="M45 18 C51 17 55 13 57 5 C62 12 62 22 55 25 C51 27 47 27 45 26 Z" fill="${accent}"/>`

// C — Torchman. The appliance, as literally as the letterform allows: long thin
// nozzle well past the F's width, trigger wedge, flared grip, short hard burst
// at the muzzle the way a piezo lighter actually fires.
const torchman = (fg, accent) => `
  <path d="M12 16 H24 V50 L28 60 H8 L12 50 Z" fill="${fg}"/>
  <rect x="12" y="16" width="20" height="12" fill="${fg}"/>
  <rect x="32" y="19" width="22" height="6" fill="${fg}"/>
  <path d="M12 34 H26 L32 40 L26 46 H12 Z" fill="${fg}"/>
  <path d="M53 19 C57 18 59 15 60 9 C63 14 64 21 58 24 C56 25 54 24 53 23 Z" fill="${accent}"/>`

const BODIES = [
  {
    id: 'nozzle',
    label: 'A · Nozzle — letter-first',
    draw: nozzle,
    note:
      'The letter survives, the lighter does not.\n' +
      'A muzzle 2 units narrower than the\n' +
      '12-weight bar is invisible at 512px. The\n' +
      'flame also hooks back left over the bar.',
  },
  {
    id: 'trigger',
    label: 'B · Trigger',
    draw: trigger,
    note:
      'The flared foot reads as a lamp base, not\n' +
      'a grip — it is the widest thing in the\n' +
      'drawing and the eye goes there first.',
  },
  {
    id: 'torchman',
    label: 'C · Torchman — appliance-first',
    draw: torchman,
    note:
      'The only one that reads as the object,\n' +
      'and it reads as a *gun*, which is the one\n' +
      'read to avoid. The 6-unit nozzle also\n' +
      'lands on 1.5px at 16 and smears to grey.',
  },
]

// --- 4b: the shape of the jet ------------------------------------------------
// Same body under all three. The risk this sheet exists to test: fire blown
// sideways off a bar is one small step from a speed-line swoosh, which is
// exactly what killed round 3c's first tongue.
const body = (fg) => `
  <rect x="12" y="16" width="12" height="44" fill="${fg}"/>
  <rect x="12" y="16" width="24" height="12" fill="${fg}"/>
  <rect x="36" y="17" width="8"  height="10" fill="${fg}"/>
  <rect x="12" y="34" width="22" height="12" fill="${fg}"/>`

const JETS = [
  {
    id: 'flat',
    label: 'A · Flat — along the barrel',
    path: 'M43 18 C50 17 55 15 58 12 C60 10 62 9 63 8 C62 14 60 19 56 23 C52 27 47 28 43 27 Z',
    note:
      'A thin crescent lying on the barrel. This\n' +
      'is a swoosh, not fire — the same failure\n' +
      'that killed round 3c.',
  },
  {
    id: 'rake',
    label: 'B · Rake — 45° off the muzzle',
    path: 'M43 18 C49 15 53 11 56 6 C57 4 58 2 59 0 C60 8 59 15 55 20 C51 25 47 27 43 27 Z',
    note:
      'A tapered blade. Reads as a leaf at 96px\n' +
      'and as a motion streak at 16.',
  },
  {
    id: 'lick',
    label: 'C · Lick — belly, then a tip',
    path:
      'M43 18 C48 16 51 13 53 9 C55 5 56 3 57 1 C58 7 58 12 57 16 ' +
      'C60 14 62 12 63 10 C62 17 59 23 54 26 C50 28 46 28 43 27 Z',
    note:
      'The only one that reads as fire, and the\n' +
      'notch between the belly and the second\n' +
      'tongue is what does it. This is the jet\n' +
      'that ships.',
  },
].map(({ id, label, path, note }) => ({
  id,
  label,
  note,
  draw: (fg, accent) => `${body(fg)}\n  <path d="${path}" fill="${accent}"/>`,
}))

// --- 4c: how the fire leaves the muzzle --------------------------------------
// 4b's lick is the only jet that reads as fire, but at app-icon size the flame
// covers the nozzle completely and the lighter disappears — what is left is an
// F with a flame on it, which is round 3 again. So: does the muzzle need to be
// visible, and how much daylight can sit between it and the fire before the
// flame starts floating free (which is Firebase's silhouette, not ours)?
const LICK =
  'M43 18 C48 16 51 13 53 9 C55 5 56 3 57 1 C58 7 58 12 57 16 ' +
  'C60 14 62 12 63 10 C62 17 59 23 54 26 C50 28 46 28 43 27 Z'

// barrel: where the 12-weight top bar stops. muzzle: where the narrow nozzle
// stops. shift: how far right of the muzzle the flame's base sits.
const muzzled = ({ barrel, muzzle, shift }) => (fg, accent) => `
  <rect x="12" y="16" width="12" height="44" fill="${fg}"/>
  <rect x="12" y="16" width="${barrel - 12}" height="12" fill="${fg}"/>
  <rect x="${barrel}" y="17" width="${muzzle - barrel}" height="10" fill="${fg}"/>
  <rect x="12" y="34" width="22" height="12" fill="${fg}"/>
  <g transform="translate(${shift} 0)"><path d="${LICK}" fill="${accent}"/></g>`

const MUZZLES = [
  {
    id: 'flush',
    label: 'A · Flush — fire on the bar',
    draw: muzzled({ barrel: 36, muzzle: 44, shift: 0 }),
    note:
      'The flame covers the whole end of the\n' +
      'bar. No muzzle, so no lighter: this is\n' +
      'round 3 with the fire pointed sideways.',
  },
  {
    id: 'proud',
    label: 'B · Proud — muzzle shows',
    draw: muzzled({ barrel: 32, muzzle: 44, shift: 2 }),
    note:
      'It does not show. Two units against the\n' +
      'bar is under half a pixel at 32 and still\n' +
      'invisible at 512 — the whole axis is too\n' +
      'fine to matter, which is why 4d exists.',
  },
  {
    id: 'gap',
    label: 'C · Gap — fire off the muzzle',
    draw: muzzled({ barrel: 30, muzzle: 42, shift: 4 }),
    note:
      'Three units of daylight. At 16px the thin\n' +
      'nozzle is a grey sliver that reads as an\n' +
      'artifact, and the flame starts to float\n' +
      'free — the silhouette we are avoiding.',
  },
]

// --- 4d: which lighter cues survive ------------------------------------------
// 4c's answer was "none of them": a muzzle 2 units narrower than the bar is
// invisible at 512px, let alone at 16. If the object is going to be read at
// all, the cues have to be coarse — a muzzle at half the bar's weight, and the
// middle arm cut as a trigger rather than left as a bar. This sheet is the cost
// of each cue, measured at the size where it is paid.
const cued = ({ barrel, muzzle, weight, shift, arm }) => (fg, accent) => {
  const y = 16 + (12 - weight) / 2
  const trigger =
    arm === 'wedge'
      ? `<path d="M12 34 H28 L33 40 L28 46 H12 Z" fill="${fg}"/>`
      : `<rect x="12" y="34" width="22" height="12" fill="${fg}"/>`
  return `
  <rect x="12" y="16" width="12" height="44" fill="${fg}"/>
  <rect x="12" y="16" width="${barrel - 12}" height="12" fill="${fg}"/>
  ${muzzle > barrel ? `<rect x="${barrel}" y="${y}" width="${muzzle - barrel}" height="${weight}" fill="${fg}"/>` : ''}
  ${trigger}
  <g transform="translate(${shift} 0)"><path d="${LICK}" fill="${accent}"/></g>`
}

const CUES = [
  {
    id: 'plain',
    label: 'A · No cue — F on fire',
    draw: cued({ barrel: 44, muzzle: 44, weight: 12, shift: 0, arm: 'bar' }),
    note:
      'Perfectly legible, and it says nothing\n' +
      'about a lighter. Kept as the control: if\n' +
      'the cues cost more than this, drop them.',
  },
  {
    id: 'spout',
    label: 'B · Half-weight muzzle',
    draw: cued({ barrel: 32, muzzle: 46, weight: 6, shift: 3, arm: 'bar' }),
    note:
      'The object reads from 32px up and the F\n' +
      'is untouched. One coarse cue is enough,\n' +
      'and it is the one that costs nothing at\n' +
      'the bottom of the size range. This ships.',
  },
  {
    id: 'both',
    label: 'C · Muzzle + trigger arm',
    draw: cued({ barrel: 32, muzzle: 46, weight: 6, shift: 3, arm: 'wedge' }),
    note:
      'The wedge does not read as a trigger — it\n' +
      'reads as a fast-forward triangle, and at\n' +
      '16px it fills the counter. One cue too\n' +
      'many.',
  },
]

// --- sheets ------------------------------------------------------------------

// Wider than sheet.mjs's default: at 400 the notes ran off the column edge.
const COL = 470
const DARK = { fg: PAPER, accent: EMBER, bg: INK, muted: MUTED, col: COL }
const LIGHT = { fg: INK, accent: EMBER_DEEP, bg: PAPER, muted: '#5A6782', col: COL }
const SIZE_CAPTION = '96 / 48 / 32 / 16 px, actual size'

const SHEETS = [
  {
    file: 'bodies-dark',
    options: BODIES,
    ...DARK,
    caption: `Round 4a — F as a trigger lighter   ·   ${SIZE_CAPTION}   ·   on ink, ember #FF5A2B`,
  },
  {
    file: 'bodies-light',
    options: BODIES,
    ...LIGHT,
    caption: `Round 4a — F as a trigger lighter   ·   ${SIZE_CAPTION}   ·   on paper, ember-deep #C2410C`,
  },
  {
    file: 'jets-dark',
    options: JETS,
    ...DARK,
    caption: `Round 4b — the shape of the jet   ·   ${SIZE_CAPTION}   ·   on ink`,
  },
  {
    file: 'jets-light',
    options: JETS,
    ...LIGHT,
    caption: `Round 4b — the shape of the jet   ·   ${SIZE_CAPTION}   ·   on paper`,
  },
  {
    file: 'muzzle-dark',
    options: MUZZLES,
    ...DARK,
    caption: `Round 4c — how the fire leaves the muzzle   ·   ${SIZE_CAPTION}   ·   on ink`,
  },
  {
    file: 'muzzle-light',
    options: MUZZLES,
    ...LIGHT,
    caption: `Round 4c — how the fire leaves the muzzle   ·   ${SIZE_CAPTION}   ·   on paper`,
  },
  {
    file: 'cues-dark',
    options: CUES,
    ...DARK,
    caption: `Round 4d — which lighter cues survive   ·   ${SIZE_CAPTION}   ·   on ink`,
  },
  {
    file: 'cues-light',
    options: CUES,
    ...LIGHT,
    caption: `Round 4d — which lighter cues survive   ·   ${SIZE_CAPTION}   ·   on paper`,
  },
]

const out = join(root, 'lighter')
mkdirSync(out, { recursive: true })

for (const config of SHEETS) {
  const source = sheet(config)
  const width = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const file = join(out, `${config.file}.png`)
  writeFileSync(file, renderPng(source, width))
  console.log(file)
}

// Each option standing alone at the size that decides it.
for (const option of [...BODIES, ...JETS, ...MUZZLES, ...CUES]) {
  const tiny = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" fill="${INK}"/>${option.draw(PAPER, EMBER)}</svg>`
  for (const size of [16, 512]) {
    const file = join(out, `${option.id}-${size}.png`)
    writeFileSync(file, renderPng(tiny, size))
    console.log(file)
  }
}
