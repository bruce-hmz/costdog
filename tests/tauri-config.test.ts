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

test('desktop release versions stay aligned', () => {
  const cargoVersion = cargoManifest.match(/^version = "([^"]+)"$/m)?.[1];

  assert.equal(tauriConfig.version, packageJson.version);
  assert.equal(cargoVersion, packageJson.version);
});
