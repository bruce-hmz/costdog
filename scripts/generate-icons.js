const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

// Dog icon: a simple Shiba-style dog face on dark background
function createDogIcon(size) {
  const pixels = Buffer.alloc(size * size * 4); // RGBA

  function setPixel(x, y, r, g, b, a = 255) {
    x = Math.round(x); y = Math.round(y);
    if (x < 0 || x >= size || y < 0 || y >= size) return;
    const i = (y * size + x) * 4;
    pixels[i] = r; pixels[i+1] = g; pixels[i+2] = b; pixels[i+3] = a;
  }

  function fillCircle(cx, cy, radius, r, g, b) {
    for (let y = -radius; y <= radius; y++)
      for (let x = -radius; x <= radius; x++)
        if (x*x + y*y <= radius*radius) setPixel(cx+x, cy+y, r, g, b);
  }

  function fillEllipse(cx, cy, rx, ry, r, g, b) {
    for (let y = -ry; y <= ry; y++)
      for (let x = -rx; x <= rx; x++)
        if ((x*x)/(rx*rx) + (y*y)/(ry*ry) <= 1) setPixel(cx+x, cy+y, r, g, b);
  }

  function fillRoundRect(rx, ry, rw, rh, radius, r, g, b) {
    for (let y = ry; y < ry + rh; y++)
      for (let x = rx; x < rx + rw; x++) {
        const dx = Math.max(rx+radius-x, 0, x-(rx+rw-radius));
        const dy = Math.max(ry+radius-y, 0, y-(ry+rh-radius));
        if (dx*dx+dy*dy <= radius*radius) setPixel(x, y, r, g, b);
      }
  }

  const s = size / 128;

  // Background
  fillRoundRect(0, 0, size, size, Math.round(20*s), 22, 22, 35);

  // Dog face (orange/tan) - main circle
  const cx = 64*s, cy = 68*s;
  fillCircle(cx, cy, 36*s, 218, 165, 82);

  // White muzzle area
  fillEllipse(cx, cy+12*s, 20*s, 18*s, 245, 240, 230);

  // Left ear (triangle-ish, dark)
  fillEllipse(cx-28*s, cy-28*s, 14*s, 20*s, 180, 120, 50);
  // Inner ear
  fillEllipse(cx-28*s, cy-26*s, 8*s, 12*s, 218, 165, 82);

  // Right ear
  fillEllipse(cx+28*s, cy-28*s, 14*s, 20*s, 180, 120, 50);
  fillEllipse(cx+28*s, cy-26*s, 8*s, 12*s, 218, 165, 82);

  // Forehead marking (white stripe)
  fillEllipse(cx, cy-18*s, 10*s, 14*s, 245, 240, 230);

  // Left eye
  fillCircle(cx-14*s, cy-6*s, 5*s, 30, 30, 30);
  // Left eye highlight
  fillCircle(cx-12*s, cy-8*s, 2*s, 255, 255, 255);

  // Right eye
  fillCircle(cx+14*s, cy-6*s, 5*s, 30, 30, 30);
  // Right eye highlight
  fillCircle(cx+16*s, cy-8*s, 2*s, 255, 255, 255);

  // Nose
  fillEllipse(cx, cy+6*s, 6*s, 4*s, 30, 30, 30);
  // Nose highlight
  fillEllipse(cx-1*s, cy+5*s, 2*s, 1*s, 80, 80, 80);

  // Mouth
  for (let x = -4*s; x <= 4*s; x++) {
    const my = cy + 12*s + Math.abs(x) * 0.4;
    setPixel(cx + x, my, 30, 30, 30);
    setPixel(cx + x, my + 1, 30, 30, 30);
  }

  // Tongue (small pink)
  fillEllipse(cx, cy+16*s, 3*s, 4*s, 230, 130, 130);

  // Cheek blush
  fillEllipse(cx-24*s, cy+4*s, 6*s, 4*s, 240, 180, 140);
  fillEllipse(cx+24*s, cy+4*s, 6*s, 4*s, 240, 180, 140);

  return pixels;
}

// macOS menu-bar template icon: a dog-head silhouette drawn in black with alpha only.
// The system recolors template images for light/dark menu bars and discards any color,
// so the app icon (an opaque rounded square) would render as a solid blob here.
function createTrayTemplate(size) {
  const pixels = Buffer.alloc(size * size * 4);
  const s = size / 22; // shapes below are authored on a 22pt menu-bar grid
  const SAMPLES = 4;

  function inEllipse(x, y, cx, cy, rx, ry) {
    const dx = (x - cx) / rx, dy = (y - cy) / ry;
    return dx * dx + dy * dy <= 1;
  }

  function inTriangle(x, y, ax, ay, bx, by, cx, cy) {
    const d1 = (x - bx) * (ay - by) - (ax - bx) * (y - by);
    const d2 = (x - cx) * (by - cy) - (bx - cx) * (y - cy);
    const d3 = (x - ax) * (cy - ay) - (cx - ax) * (y - ay);
    const hasNeg = d1 < 0 || d2 < 0 || d3 < 0;
    const hasPos = d1 > 0 || d2 > 0 || d3 > 0;
    return !(hasNeg && hasPos);
  }

  function covered(x, y) {
    const body =
      inEllipse(x, y, 11, 13.5, 7.6, 6.8) ||
      inTriangle(x, y, 2.6, 1.4, 9.2, 7.6, 3.2, 11.4) ||
      inTriangle(x, y, 19.4, 1.4, 12.8, 7.6, 18.8, 11.4);
    if (!body) return false;
    // Knocked-out features keep the silhouette readable at 22pt.
    if (inEllipse(x, y, 8.2, 12.2, 1.3, 1.5)) return false;
    if (inEllipse(x, y, 13.8, 12.2, 1.3, 1.5)) return false;
    if (inEllipse(x, y, 11, 16.4, 1.9, 1.4)) return false;
    return true;
  }

  for (let py = 0; py < size; py++) {
    for (let px = 0; px < size; px++) {
      let hits = 0;
      for (let sy = 0; sy < SAMPLES; sy++) {
        for (let sx = 0; sx < SAMPLES; sx++) {
          const x = (px + (sx + 0.5) / SAMPLES) / s;
          const y = (py + (sy + 0.5) / SAMPLES) / s;
          if (covered(x, y)) hits++;
        }
      }
      const i = (py * size + px) * 4;
      pixels[i] = 0; pixels[i + 1] = 0; pixels[i + 2] = 0;
      pixels[i + 3] = Math.round((hits / (SAMPLES * SAMPLES)) * 255);
    }
  }

  return pixels;
}

function createPNG(width, height, pixels) {
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0); ihdr.writeUInt32BE(height, 4);
  ihdr.writeUInt8(8, 8); ihdr.writeUInt8(6, 9); // 8-bit RGBA

  const raw = Buffer.alloc(height * (1 + width * 4));
  for (let y = 0; y < height; y++) {
    raw[y * (1 + width * 4)] = 0;
    for (let x = 0; x < width; x++) {
      const si = (y * width + x) * 4;
      const di = y * (1 + width * 4) + 1 + x * 4;
      raw[di] = pixels[si]; raw[di+1] = pixels[si+1]; raw[di+2] = pixels[si+2]; raw[di+3] = pixels[si+3];
    }
  }

  const compressed = zlib.deflateSync(raw);
  function chunk(type, data) {
    const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
    const t = Buffer.from(type, 'ascii');
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(Buffer.concat([t, data])));
    return Buffer.concat([len, t, data, crc]);
  }

  return Buffer.concat([signature, chunk('IHDR', ihdr), chunk('IDAT', compressed), chunk('IEND', Buffer.alloc(0))]);
}

function crc32(buf) {
  let c = 0xFFFFFFFF;
  for (let i = 0; i < buf.length; i++) { c ^= buf[i]; for (let j = 0; j < 8; j++) c = (c&1)?((c>>>1)^0xEDB88320):(c>>>1); }
  return (c ^ 0xFFFFFFFF) >>> 0;
}

const iconsDir = path.join(__dirname, '..', 'src-tauri', 'icons');
for (const size of [32, 128, 256]) {
  const png = createPNG(size, size, createDogIcon(size));
  fs.writeFileSync(path.join(iconsDir, `${size}x${size}.png`), png);
  console.log(`${size}x${size}.png`);
}

// Menu-bar template icon at @2x (44px downscales cleanly to the 22pt menu bar).
// The .rgba file is what the tray embeds — Tauri's Image::new takes raw RGBA, so
// shipping it pre-decoded avoids pulling a PNG codec in just for this icon. The .png
// is the reviewable copy: it is the only form a human can actually look at.
const trayPixels = createTrayTemplate(44);
fs.writeFileSync(path.join(iconsDir, 'tray-template.rgba'), trayPixels);
fs.writeFileSync(path.join(iconsDir, 'tray-template.png'), createPNG(44, 44, trayPixels));
console.log('tray-template.rgba + tray-template.png');

// ICO
const png32 = fs.readFileSync(path.join(iconsDir, '32x32.png'));
const h = Buffer.alloc(6); h.writeUInt16LE(0,0); h.writeUInt16LE(1,2); h.writeUInt16LE(1,4);
const e = Buffer.alloc(16); e.writeUInt8(32,0); e.writeUInt8(32,1); e.writeUInt16LE(1,4); e.writeUInt16LE(32,6);
e.writeUInt32LE(png32.length,8); e.writeUInt32LE(22,12);
fs.writeFileSync(path.join(iconsDir, 'icon.ico'), Buffer.concat([h, e, png32]));
console.log('icon.ico');

// Favicon
const pubDir = path.join(__dirname, '..', 'src', 'web', 'public');
if (fs.existsSync(pubDir)) { fs.copyFileSync(path.join(iconsDir, '32x32.png'), path.join(pubDir, 'favicon.png')); console.log('favicon.png'); }
console.log('Done!');
