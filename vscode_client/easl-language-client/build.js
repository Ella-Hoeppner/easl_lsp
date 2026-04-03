const fs = require('fs');
const path = require('path');

if (!fs.existsSync('out')) {
  fs.mkdirSync('out');
}

// Copy the compiled Rust binary into the extension output directory.
const binaryName = process.platform === 'win32' ? 'easl_lsp.exe' : 'easl_lsp';
const src = path.join(__dirname, '..', '..', 'target', 'release', binaryName);
const dest = path.join('out', binaryName);

if (!fs.existsSync(src)) {
  console.error(`ERROR: Binary not found at ${src}`);
  console.error('Run `cargo build --release --bin easl_lsp` first.');
  process.exit(1);
}

fs.copyFileSync(src, dest);
// Make it executable on Unix.
if (process.platform !== 'win32') {
  fs.chmodSync(dest, 0o755);
}
console.log(`Copied ${binaryName} to out/`);
