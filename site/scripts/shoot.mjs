#!/usr/bin/env node
// Renders the site in a real browser at desktop and phone widths, and checks the
// things a screenshot cannot show: that every asset the page asks for exists,
// that nothing logged an error, and that no element overflows the viewport.
//
//   cd site && npm run shoot        (needs: npx puppeteer browsers install chrome)
//
// Screenshots land in site/.shots/, which is not committed — they are a check,
// not a deliverable.
import puppeteer from 'puppeteer'
import { createServer } from 'node:http'
import { readFile, mkdir } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { dirname, join, extname, normalize } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')
const shots = join(root, '.shots')

const TYPES = {
  '.html': 'text/html',
  '.css': 'text/css',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.webmanifest': 'application/manifest+json',
}

const serve = () =>
  new Promise((resolve) => {
    const server = createServer(async (req, res) => {
      const path = normalize(decodeURI(req.url.split('?')[0]))
      const file = join(root, path === '/' ? 'index.html' : path)
      try {
        const body = await readFile(file)
        res.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream' })
        res.end(body)
      } catch {
        res.writeHead(404).end('not found')
      }
    })
    server.listen(0, '127.0.0.1', () => resolve(server))
  })

const VIEWS = [
  { name: 'desktop', width: 1280, height: 900 },
  { name: 'phone', width: 390, height: 844, deviceScaleFactor: 2 },
]

const server = await serve()
const base = `http://127.0.0.1:${server.address().port}/`
await mkdir(shots, { recursive: true })

const problems = []
const browser = await puppeteer.launch({ args: ['--no-sandbox'] })

for (const view of VIEWS) {
  const page = await browser.newPage()
  await page.setViewport(view)
  page.on('console', (m) => m.type() === 'error' && problems.push(`console error (${view.name}): ${m.text()}`))
  page.on('pageerror', (e) => problems.push(`page error (${view.name}): ${e.message}`))
  page.on('requestfailed', (r) => problems.push(`failed request (${view.name}): ${r.url()}`))
  page.on('response', (r) => {
    if (r.status() >= 400) problems.push(`HTTP ${r.status()} (${view.name}): ${r.url()}`)
  })

  await page.goto(base, { waitUntil: 'networkidle0' })

  // Anything wider than the viewport means a horizontal scrollbar on a phone.
  // Content inside a deliberately scrollable box (a terminal block, the matrix)
  // is allowed to be wider than the screen, so those subtrees are skipped.
  const overflow = await page.evaluate(() => {
    const scrollable = (el) => {
      for (let node = el.parentElement; node; node = node.parentElement) {
        const x = getComputedStyle(node).overflowX
        if (x === 'auto' || x === 'scroll') return true
      }
      return false
    }
    const wide = [...document.querySelectorAll('body *')].filter(
      (el) =>
        el.getBoundingClientRect().right > document.documentElement.clientWidth + 1 && !scrollable(el),
    )
    return wide.slice(0, 5).map((el) => `${el.tagName.toLowerCase()}.${el.className || '(no class)'}`)
  })
  for (const el of overflow) problems.push(`overflows viewport (${view.name}): ${el}`)

  await page.screenshot({ path: join(shots, `${view.name}.png`), fullPage: true })
  await page.screenshot({ path: join(shots, `${view.name}-fold.png`) })
  await page.close()
  console.log(`ok   ${view.name} ${view.width}x${view.height}`)
}

await browser.close()
server.close()

if (problems.length) {
  console.log(`\n${problems.length} problem(s):`)
  for (const p of problems) console.log(`FAIL ${p}`)
  process.exit(1)
}
console.log('\nno console errors, no failed requests, nothing overflowing')
