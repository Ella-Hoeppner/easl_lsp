const fs = require('fs')
const path = require('path')

if (!fs.existsSync('out')) {
  fs.mkdirSync('out')
}

const binaryName = process.platform === 'win32' ? 'sse_lsp.exe' : 'sse_lsp'

fs.copyFileSync(
  path.join('..', '..', 'target', 'release', binaryName),
  path.join('out', binaryName)
)
