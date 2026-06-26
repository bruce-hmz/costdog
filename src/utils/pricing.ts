import * as fs from 'fs';
import * as path from 'path';
import { getCostDogConfigDir } from './paths';
import { ModelPricing } from '../types';

const PRICING_CACHE_FILE = 'pricing-cache.json';
const PRICING_URL = 'https://openrouter.ai/api/v1/models';
const CACHE_TTL_MS = 24 * 60 * 60 * 1000; // 24 hours

// Fallback prices for models not on OpenRouter (per million tokens)
const FALLBACK_PRICING: Record<string, { input: number; output: number }> = {
  'mimo-v2.5-pro': { input: 0.5, output: 2.0 },
  'glm-5.1': { input: 1.0, output: 3.0 },
  'glm-5.2': { input: 0.95, output: 3.0 },
};

/**
 * Fetch pricing from OpenRouter API
 */
async function fetchOpenRouterPricing(): Promise<ModelPricing[]> {
  try {
    const resp = await fetch(PRICING_URL);
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const data = await resp.json() as any;

    const models: ModelPricing[] = [];
    for (const m of data.data || []) {
      // OpenRouter pricing is per token, convert to per million
      const inputPrice = (m.pricing?.prompt || 0) * 1_000_000;
      const outputPrice = (m.pricing?.completion || 0) * 1_000_000;

      models.push({
        modelId: m.id,
        displayName: m.name || m.id,
        provider: m.id?.split('/')[0] || 'unknown',
        inputPricePerMToken: inputPrice,
        outputPricePerMToken: outputPrice,
        lastUpdated: new Date().toISOString(),
      });
    }
    return models;
  } catch (e) {
    console.warn('[CostDog] OpenRouter pricing fetch failed:', e instanceof Error ? e.message : e);
    return [];
  }
}

/**
 * Load pricing from cache or fetch fresh
 */
export async function loadPricing(): Promise<ModelPricing[]> {
  const configDir = getCostDogConfigDir();
  const cachePath = path.join(configDir, PRICING_CACHE_FILE);

  // Try cache first
  if (fs.existsSync(cachePath)) {
    try {
      const cached = JSON.parse(fs.readFileSync(cachePath, 'utf-8'));
      const age = Date.now() - new Date(cached.fetchedAt).getTime();
      if (age < CACHE_TTL_MS && cached.models?.length > 0) {
        return cached.models;
      }
    } catch {}
  }

  // Fetch fresh
  const models = await fetchOpenRouterPricing();
  if (models.length > 0) {
    try {
      fs.mkdirSync(configDir, { recursive: true });
      fs.writeFileSync(cachePath, JSON.stringify({ models, fetchedAt: new Date().toISOString() }, null, 2));
    } catch {}
    return models;
  }

  // Fallback: return cached even if stale
  if (fs.existsSync(cachePath)) {
    try {
      const cached = JSON.parse(fs.readFileSync(cachePath, 'utf-8'));
      if (cached.models?.length > 0) return cached.models;
    } catch {}
  }

  // Last resort: return fallback
  console.warn('[CostDog] pricing unavailable (fetch failed, no usable cache) — using fallback pricing');
  return Object.entries(FALLBACK_PRICING).map(([id, p]) => ({
    modelId: id,
    displayName: id,
    provider: id.split('/')[0] || 'unknown',
    inputPricePerMToken: p.input,
    outputPricePerMToken: p.output,
    lastUpdated: 'fallback',
  }));
}

/**
 * Find price for a model, with fuzzy matching
 */
export function findModelPrice(modelId: string, pricing: ModelPricing[]): { input: number; output: number } | null {
  if (!modelId) return null;

  const lower = modelId.toLowerCase();

  // Exact match
  for (const m of pricing) {
    if (m.modelId.toLowerCase() === lower) {
      return { input: m.inputPricePerMToken, output: m.outputPricePerMToken };
    }
  }

  // Match by suffix (e.g., "claude-opus-4-8" matches "anthropic/claude-opus-4-8")
  for (const m of pricing) {
    const suffix = m.modelId.split('/').pop()?.toLowerCase();
    if (suffix === lower) {
      return { input: m.inputPricePerMToken, output: m.outputPricePerMToken };
    }
  }

  // Normalize dashes to dots (claude-opus-4-8 -> claude-opus-4.8)
  const normalized = lower.replace(/(\d+)-(\d+)/g, '$1.$2');
  for (const m of pricing) {
    const mLower = m.modelId.toLowerCase();
    const suffix = mLower.split('/').pop();
    if (mLower === normalized || suffix === normalized) {
      return { input: m.inputPricePerMToken, output: m.outputPricePerMToken };
    }
  }

  // Fuzzy: contains match
  for (const m of pricing) {
    const mLower = m.modelId.toLowerCase();
    if (mLower.includes(lower) || lower.includes(mLower) || mLower.includes(normalized) || normalized.includes(mLower)) {
      return { input: m.inputPricePerMToken, output: m.outputPricePerMToken };
    }
  }

  // Check fallback
  for (const [id, p] of Object.entries(FALLBACK_PRICING)) {
    if (lower.includes(id.toLowerCase()) || id.toLowerCase().includes(lower)) {
      return { input: p.input, output: p.output };
    }
  }

  return null;
}

/**
 * Calculate cost for token usage
 */
export function calculateCost(
  inputTokens: number,
  outputTokens: number,
  cacheReadTokens: number,
  cacheCreationTokens: number,
  reasoningTokens: number,
  modelId: string,
  pricing: ModelPricing[],
): number {
  const price = findModelPrice(modelId, pricing);
  if (!price) return 0;

  const perM = 1_000_000;
  // inputTokens is the NON-cached portion: Anthropic reports it separately from cache
  // read/creation, and the Codex parser subtracts cached tokens. So no subtraction here.
  return (
    (inputTokens / perM) * price.input +
    (cacheReadTokens / perM) * (price.input * 0.1) +
    (cacheCreationTokens / perM) * (price.input * 1.25) +
    (outputTokens / perM) * price.output +
    (reasoningTokens / perM) * price.output
  );
}
