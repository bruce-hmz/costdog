import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const tauriConfig = JSON.parse(
  readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'),
);
const desktopHtml = readFileSync(
  new URL('../src-tauri/embedded/index.html', import.meta.url),
  'utf8',
);
const packageJson = JSON.parse(
  readFileSync(new URL('../package.json', import.meta.url), 'utf8'),
);
const cargoManifest = readFileSync(
  new URL('../src-tauri/Cargo.toml', import.meta.url),
  'utf8',
);

test('desktop CSP allows Tauri IPC commands', () => {
  const csp = tauriConfig.app.security.csp;

  assert.match(csp, /connect-src[^;]*\bipc:/);
  assert.match(csp, /connect-src[^;]*http:\/\/ipc\.localhost/);
  assert.doesNotMatch(desktopHtml, /\son[a-z]+\s*=/i);
  assert.match(desktopHtml, /addEventListener\('click',event=>/);
  assert.match(desktopHtml, /data-action="toggle"/);
});

// The bar is laid out at a fixed 410px (five skins are drawn against that width), and
// setup() re-applies 410x36 on every launch. Leaving the window resizable let the user
// drag it wider than the bar, exposing a strip of empty background on the right.
test('bar window is not resizable while its layout is fixed-width', () => {
  const [mainWindow] = tauriConfig.app.windows;
  const capabilities = JSON.parse(
    readFileSync(new URL('../src-tauri/capabilities/default.json', import.meta.url), 'utf8'),
  );

  assert.match(desktopHtml, /\.bar\{[^}]*width:410px/);
  assert.equal(mainWindow.width, 410);
  assert.equal(mainWindow.resizable, false);
  assert.ok(!capabilities.permissions.includes('core:window:allow-start-resize-dragging'));
});

test('desktop release versions stay aligned', () => {
  const cargoVersion = cargoManifest.match(/^version = "([^"]+)"$/m)?.[1];

  assert.equal(tauriConfig.version, packageJson.version);
  assert.equal(cargoVersion, packageJson.version);
});
