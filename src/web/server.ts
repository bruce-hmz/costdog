import express from 'express';
import * as path from 'path';
import { fullScan, getDashboardData } from '../aggregator';
import { loadPricing } from '../utils/pricing';

const app = express();
const PORT = process.env.COSTDOG_PORT || 3456;

app.use(express.static(path.join(__dirname, 'public')));

// API: Full scan and return dashboard data
app.get('/api/dashboard', async (req, res) => {
  try {
    await fullScan();
    const data = getDashboardData();
    res.json(data);
  } catch (err: any) {
    res.status(500).json({ error: err.message });
  }
});

// API: Scan only
app.post('/api/scan', async (req, res) => {
  try {
    const result = await fullScan();
    res.json(result);
  } catch (err: any) {
    res.status(500).json({ error: err.message });
  }
});

// API: Pricing
app.get('/api/pricing', async (req, res) => {
  try {
    const pricing = await loadPricing();
    res.json(pricing);
  } catch (err: any) {
    res.status(500).json({ error: err.message });
  }
});

export function startWebServer(port?: number) {
  const p = port ?? Number(PORT);
  const server = app.listen(p, '127.0.0.1', () => {
    const address = server.address();
    const listeningPort = typeof address === 'object' && address ? address.port : p;
    console.log(`🐕 CostDog web dashboard: http://127.0.0.1:${listeningPort}`);
  });
  return server;
}
