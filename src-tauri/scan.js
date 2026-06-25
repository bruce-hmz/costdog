#!/usr/bin/env node

/**
 * CostDog Scanner - Scans Claude Code and Codex sessions
 * This script can be called from the Tauri app to update the database
 */

const path = require('path');
const os = require('os');
const fs = require('fs');

// Import the existing scanner modules
const { scanClaudeSessions } = require('../src/parsers/claude-code');
const { scanCodexSessions } = require('../src/parsers/codex');
const { loadPricing, calculateCost } = require('../src/utils/pricing');
const { upsertSession, getAggregateStats, getTopModels, getRecentSessions, getAlerts, addAlert } = require('../src/db/schema');

async function main() {
    try {
        console.log('🐕 CostDog Scanner starting...');

        const pricing = await loadPricing();

        const claudeSessions = scanClaudeSessions();
        const codexSessions = scanCodexSessions();
        const allSessions = [...claudeSessions, ...codexSessions];

        let newCount = 0;
        for (const s of allSessions) {
            const cost = calculateCost(
                s.tokenUsage.inputTokens,
                s.tokenUsage.outputTokens,
                s.tokenUsage.cacheReadTokens,
                s.model,
                pricing,
            );

            upsertSession({
                sessionId: s.sessionId,
                source: s.source,
                model: s.model,
                project: s.project,
                startTime: s.startTime,
                endTime: s.endTime,
                inputTokens: s.tokenUsage.inputTokens,
                outputTokens: s.tokenUsage.outputTokens,
                cacheReadTokens: s.tokenUsage.cacheReadTokens,
                cacheCreationTokens: s.tokenUsage.cacheCreationTokens,
                reasoningOutputTokens: s.tokenUsage.reasoningOutputTokens,
                diskWriteBytes: s.diskWriteBytes,
                cost,
            });
            newCount++;
        }

        console.log(`✅ Scan complete: ${newCount} sessions processed`);
        process.exit(0);
    } catch (error) {
        console.error('❌ Scan failed:', error.message);
        process.exit(1);
    }
}

main();
