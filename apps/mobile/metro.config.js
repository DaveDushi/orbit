const path = require('node:path')
const { getDefaultConfig } = require('expo/metro-config')
const { withUniwindConfig } = require('uniwind/metro')

const config = getDefaultConfig(__dirname)

// The catalog of the agent's cards, shared with the sidecar and the web (apps/agent deploys on its own).
const catalog = path.resolve(__dirname, '../agent/src/ui-catalog.ts')
// The catalog resolves its imports from apps/agent when that has node_modules: use the app's one copy.
const shared = new Set(['zod', '@json-render/core'])
config.resolver.resolveRequest = (context, moduleName, platform) => {
  if (moduleName === '@agent/ui-catalog') return { type: 'sourceFile', filePath: catalog }
  if (shared.has(moduleName) && context.originModulePath === catalog) {
    return context.resolveRequest({ ...context, originModulePath: path.join(__dirname, 'package.json') }, moduleName, platform)
  }
  return context.resolveRequest(context, moduleName, platform)
}

// Uniwind must be the outermost wrapper.
module.exports = withUniwindConfig(config, {
  cssEntryFile: './src/global.css',
  dtsFile: './src/uniwind-types.d.ts',
})
