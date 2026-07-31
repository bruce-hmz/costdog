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

// The detail panel opens into 484px of viewport. Leading with the analytics blocks pushed
// the six stat cards and the session table below the fold, so the panel had to lead with
// the numbers and fold the occasional-use analysis away.
test('detail panel leads with the numbers, not the analytics', () => {
  const at = (needle: string) => desktopHtml.indexOf(needle);
  const grid = at('<div class="grid">');
  const budget = at('Monthly budget');
  const analysis = at('id="analysis"');
  const trend = at('Cost trend');
  const donut = at('Spend by activity');
  const sessions = at('Recent Sessions');

  assert.ok(grid > 0 && budget > 0 && analysis > 0, 'all three regions exist');
  assert.ok(grid < analysis, 'stat grid renders before the analysis section');
  assert.ok(budget < analysis, 'budget renders before the analysis section');
  assert.ok(trend > analysis, 'cost trend is folded into the analysis section');
  assert.ok(donut > analysis, 'category donut is folded into the analysis section');
  assert.ok(sessions > analysis, 'recent sessions close the panel');
  // The session table scrolls inside its own box instead of stretching the panel.
  assert.match(desktopHtml, /class="session-scroll"/);
});

// The panel used to mix English section titles with Chinese controls and toasts. The bar
// skins keep their English typography (NIGHT WATCH, LAP COST) as art direction, so the
// whole file is English now — any CJK character means the two have drifted apart again.
test('desktop UI text is single-language', () => {
  const cjk = desktopHtml.match(/[一-鿿]/gu);

  assert.equal(cjk, null, `unexpected CJK text: ${cjk?.slice(0, 8).join('')}`);
  assert.match(desktopHtml, /<html lang="en"/);
});

// Icon-only controls and the range tabs have to be reachable without a mouse.
test('detail panel controls are keyboard reachable', () => {
  assert.match(desktopHtml, /class="tabs" role="tablist"/);
  assert.match(desktopHtml, /<button class="tab on" data-r="today" role="tab"/);
  assert.doesNotMatch(desktopHtml, /<div class="tab["\s]/, 'range tabs are buttons, not divs');
  assert.match(desktopHtml, /ArrowRight/, 'arrow keys move between range tabs');
  // Every bar icon button says what it does.
  const iconButtons = desktopHtml.match(/<button class="bbtn[^>]*>/g) ?? [];
  assert.ok(iconButtons.length >= 15, 'five skins x three actions');
  for (const button of iconButtons) assert.match(button, /aria-label="/);
});

// The bundled frontend talks to Rust over IPC only; the Express dashboard is a separate
// app that a packaged .app never ships, so no HTTP fallback and no port in the CSP.
test('desktop frontend has no web-server fallback', () => {
  assert.doesNotMatch(desktopHtml, /127\.0\.0\.1:3456/);
  assert.doesNotMatch(tauriConfig.app.security.csp, /3456/);
});

test('desktop release versions stay aligned', () => {
  const cargoVersion = cargoManifest.match(/^version = "([^"]+)"$/m)?.[1];

  assert.equal(tauriConfig.version, packageJson.version);
  assert.equal(cargoVersion, packageJson.version);
});
