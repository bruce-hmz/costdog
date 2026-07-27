import assert from 'node:assert/strict';
import test from 'node:test';
import { dateRange } from '../src/aggregator';

test('dateRange returns inclusive local calendar windows', () => {
  const now = new Date(2026, 6, 27, 12, 0, 0);

  assert.deepEqual(dateRange(0, now), {
    start: '2026-07-27',
    end: '2026-07-27',
  });
  assert.deepEqual(dateRange(7, now), {
    start: '2026-07-21',
    end: '2026-07-27',
  });
  assert.deepEqual(dateRange(30, now), {
    start: '2026-06-28',
    end: '2026-07-27',
  });
});
