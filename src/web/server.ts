import express from 'express';
import * as path from 'path';
import { fullScan, getDashboardData } from '../aggregator';
import { loadPricing } from '../utils/pricing';

const app = express();
const PORT = process.env.COSTDOG_PORT || 3456;

// CORS support for Tauri app
app.use((req, res, next) => {
  res.header('Access-Control-Allow-Origin', '*');
  res.header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  res.header('Access-Control-Allow-Headers', 'Content-Type');
  if (req.method === 'OPTIONS') return res.sendStatus(200);
  next();
});

app.use(express.static(path.join(__dirname, 'public')));

// Mini-dashboard for Tauri desktop widget
app.get('/mini', (req, res) => {
  res.sendFile(path.join(__dirname, '..', 'mini-dashboard', 'index.html'));
});

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
  const p = port || Number(PORT);
  app.listen(p, () => {
    console.log(`🐕 CostDog web dashboard: http://localhost:${p}`);
  });
  return app;
}
