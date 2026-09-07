import { assetLoader, onDocumentReady } from './ui/index.js'
import {
  fixDecodeWorkbench,
  fixEncodeWorkbench,
  fixFieldExplorer,
  fixFrameGallery,
  fixMessageExplorer,
  fixProjectionExplorer,
  fixRegistryEditor,
  fixRegistrySummary,
  fixSourceTable,
} from './ui/fix.js'
import { fixManifest } from './ui/yggdryl.js'

/* Thin static host for the FIX components shipped by yggdryl/ui/fix. */

const COMMAND = 'node scripts/build_docs_fix.js'

let index = null
let detail = null
const assets = assetLoader({
  baseURL: import.meta.url,
  assets: { index: 'fix.json', detail: 'fix-codes.json' },
  regenerate: COMMAND,
})

const manifest = () => {
  if (index === null) {
    index = assets.json('index').then((data) => {
      detail = assets.prefetch('detail')
      return fixManifest({ index: data })
    })
  }
  return index
}

const details = () => {
  if (detail === null) detail = assets.json('detail')
  return detail
}

const detailFailure = (error) => assets.failure({ asset: 'detail', error })

const takeFrame = () => {
  try {
    const frame = sessionStorage.getItem('ygg-fx-frame')
    if (frame !== null) sessionStorage.removeItem('ygg-fx-frame')
    return frame
  } catch {
    return null
  }
}

const openDecoder = (wire) => {
  try {
    sessionStorage.setItem('ygg-fx-frame', wire)
  } catch {
    // Navigation still works when storage is unavailable.
  }
  window.location.href = new URL('../decode/', window.location.href).href
}

const RENDERERS = {
  kpi: (model) => fixRegistrySummary({ model }),
  sources: (model) => fixSourceTable({ model }),
  fields: (model) =>
    fixFieldExplorer({
      model,
      loadDetails: details,
      renderDetailsError: detailFailure,
      pageSize: 60,
    }).element,
  messages: (model) => fixMessageExplorer({ model }).element,
  projection: (model) => fixProjectionExplorer({ model, pageSize: 60 }).element,
  decode: (model) => {
    const value = takeFrame()
    return fixDecodeWorkbench({
      model,
      loadDetails: details,
      renderDetailsError: detailFailure,
      frames: model.data.frames,
      ...(value === null ? {} : { value }),
    }).element
  },
  frames: (model) => fixFrameGallery({ model, frames: model.data.frames }).element,
  encode: (model) =>
    fixEncodeWorkbench({
      model,
      loadDetails: details,
      renderDetailsError: detailFailure,
      messageType: 'D',
      beginString: 'FIX.4.4',
      onDecode: openDecoder,
    }).element,
  registry: (model) =>
    fixRegistryEditor({
      model,
      loadDetails: details,
      renderDetailsError: detailFailure,
      pageSize: 60,
    }).element,
}

function start() {
  const roots = [...document.querySelectorAll('[data-fix]')]
  if (roots.length === 0) return
  manifest().then(
    (model) => {
      for (const root of roots) {
        const render = RENDERERS[root.dataset.fix]
        if (render === undefined) continue
        root.classList.add('ygg-ui')
        root.replaceChildren(render(model))
      }
    },
    (error) => {
      for (const root of roots) {
        root.replaceChildren(assets.failure({ asset: 'index', error }))
      }
    },
  )
}

// Material instant navigation swaps the document without reloading modules.
onDocumentReady({ run: start })
