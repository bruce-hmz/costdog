import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { once } from 'node:events';

const require = createRequire(import.meta.url);
const { startWebServer } = require('../dist/web/server.js');
const server = startWebServer(0);

try {
  await once(server, 'listening');
  const address = server.address();
  assert.equal(typeof address, 'object');
  assert.equal(address.address, '127.0.0.1');

  const origin = `http://127.0.0.1:${address.port}`;
  const response = await fetch(`${origin}/`);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get('access-control-allow-origin'), null);
  assert.match(await response.text(), /CostDog/);

  const removedMiniDashboard = await fetch(`${origin}/mini`);
  assert.equal(removedMiniDashboard.status, 404);
} finally {
  server.close();
  await once(server, 'close');
}
