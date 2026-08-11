import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

import {
  antdvChunkName,
  packageNameFromModuleId,
  vendorChunkName,
} from '../scripts/vite-vendor-chunks.mjs'

test('packageNameFromModuleId ignores pnpm peer suffixes', () => {
  assert.equal(
    packageNameFromModuleId('H:/repo/node_modules/.pnpm/antdv-next@1.4.5_vue@3.5.40/node_modules/antdv-next/dist/index.js'),
    'antdv-next',
  )
  assert.equal(
    packageNameFromModuleId('H:\\repo\\node_modules\\.pnpm\\@vue+runtime-core@3.5.40\\node_modules\\@vue\\runtime-core\\dist\\runtime-core.esm-bundler.js'),
    '@vue/runtime-core',
  )
  assert.equal(packageNameFromModuleId('H:/repo/ui/main.ts'), null)
})

test('vendorChunkName creates a small number of stable dependency groups', () => {
  assert.equal(vendorChunkName('/repo/node_modules/antdv-next/dist/button/index.js'), 'vendor-antdv-core')
  assert.equal(vendorChunkName('/repo/node_modules/antdv-next/dist/modal/index.js'), 'vendor-antdv-components')
  assert.equal(vendorChunkName('/repo/node_modules/antdv-next/dist/locale/en_US.js'), 'vendor-antdv-core')
  assert.equal(vendorChunkName('/repo/node_modules/antdv-style/dist/es/index.mjs'), 'vendor-antdv-components')
  assert.equal(vendorChunkName('/repo/node_modules/@antdv-next/icons/dist/index.js'), 'vendor-icons')
  assert.equal(vendorChunkName('/repo/node_modules/@antdv-next/cssinjs/dist/index.js'), 'vendor-vc')
  assert.equal(vendorChunkName('/repo/node_modules/@v-c/table/dist/index.js'), 'vendor-vc')
  assert.equal(vendorChunkName('/repo/node_modules/vue/dist/vue.runtime.esm-bundler.js'), 'vendor-vue')
  assert.equal(vendorChunkName('/repo/node_modules/@vue/runtime-dom/dist/runtime-dom.esm-bundler.js'), 'vendor-vue')
  assert.equal(vendorChunkName('/repo/node_modules/pinia/dist/pinia.mjs'), 'vendor-vue')
  assert.equal(vendorChunkName('/repo/node_modules/@vueuse/core/dist/index.js'), 'vendor-runtime')
  assert.equal(vendorChunkName('/repo/node_modules/es-toolkit/dist/index.js'), 'vendor-runtime')
  assert.equal(vendorChunkName('/repo/node_modules/dayjs/dayjs.min.js'), 'vendor-runtime')
  assert.equal(vendorChunkName('/repo/ui/main.ts'), undefined)
})

test('antdvChunkName keeps its internal cycle together and leaf components separate', () => {
  assert.equal(antdvChunkName('/repo/node_modules/antdv-next/dist/table/index.js'), 'vendor-antdv-core')
  assert.equal(antdvChunkName('/repo/node_modules/antdv-next/dist/config-provider/index.js'), 'vendor-antdv-core')
  assert.equal(antdvChunkName('/repo/node_modules/antdv-next/dist/modal/index.js'), 'vendor-antdv-components')
  assert.equal(antdvChunkName('/repo/node_modules/antdv-next/dist/index.js'), 'vendor-antdv-components')
  assert.equal(antdvChunkName('/repo/ui/main.ts'), undefined)
})

test('Vite production builds use the vendor chunk strategy without hiding size warnings', async () => {
  const config = await readFile(new URL('../vite.config.mjs', import.meta.url), 'utf8')
  assert.match(config, /manualChunks:\s*vendorChunkName/)
  assert.doesNotMatch(config, /chunkSizeWarningLimit/)
})

test('container builds copy the Vite chunk strategy into their UI stages', async () => {
  const dockerfiles = await Promise.all([
    readFile(new URL('../Dockerfile', import.meta.url), 'utf8'),
    readFile(new URL('../Dockerfile.ubuntu-native', import.meta.url), 'utf8'),
  ])
  for (const dockerfile of dockerfiles) {
    assert.match(
      dockerfile,
      /COPY scripts\/vite-vendor-chunks\.mjs \.\/scripts\/vite-vendor-chunks\.mjs/,
    )
  }
})
