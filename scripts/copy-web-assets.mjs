import { cpSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const source = resolve(root, 'src/web/public');
const destination = resolve(root, 'dist/web/public');

mkdirSync(destination, { recursive: true });
cpSync(source, destination, { recursive: true });
