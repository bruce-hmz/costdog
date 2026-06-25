#!/usr/bin/env node
import chalk from 'chalk';
import { fullScan, getDashboardData } from '../aggregator';
import { loadPricing, findModelPrice } from '../utils/pricing';

const VERSION = '0.1.0';

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}

function formatCost(n: number): string {
  if (n >= 100) return `$${n.toFixed(0)}`;
  if (n >= 1) return `$${n.toFixed(2)}`;
  return `$${n.toFixed(4)}`;
}

function formatBytes(n: number): string {
  if (n >= 1024 * 1024 * 1024) return `${(n / 1024 / 1024 / 1024).toFixed(1)} GB`;
  if (n >= 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${n} B`;
}

function pad(s: string, len: number): string {
  return s.padEnd(len);
}

function printSection(title: string, data: any) {
  console.log();
  console.log(chalk.bold.cyan(`┌─ ${title} ${'─'.repeat(Math.max(1, 50 - title.length))}`));
  console.log(chalk.cyan('│'));
  console.log(chalk.cyan('│') + `  Sessions:     ${chalk.bold.white(data.sessions)}`);
  console.log(chalk.cyan('│') + `  Input:        ${chalk.yellow(formatTokens(data.tokenUsage.inputTokens))} tokens`);
  console.log(chalk.cyan('│') + `  Output:       ${chalk.green(formatTokens(data.tokenUsage.outputTokens))} tokens`);
  console.log(chalk.cyan('│') + `  Cache Read:   ${chalk.gray(formatTokens(data.tokenUsage.cacheReadTokens))} tokens`);
  console.log(chalk.cyan('│') + `  Disk Writes:  ${chalk.magenta(formatBytes(data.diskWriteBytes))}`);
  console.log(chalk.cyan('│') + `  Cost:         ${chalk.bold.red(formatCost(data.cost))}`);

  if (data.topModels?.length > 0) {
    console.log(chalk.cyan('│'));
    console.log(chalk.cyan('│') + chalk.bold('  Top Models:'));
    for (const m of data.topModels.slice(0, 3)) {
      console.log(chalk.cyan('│') + `    ${pad(m.model, 25)} ${chalk.gray(`${m.calls} calls`)}  ${chalk.red(formatCost(m.cost))}`);
    }
  }
  console.log(chalk.cyan('└' + '─'.repeat(55)));
}

async function main() {
  const args = process.argv.slice(2);
  const command = args[0] || 'dashboard';

  switch (command) {
    case 'scan':
    case 's': {
      console.log(chalk.bold('🐕 CostDog — Scanning logs...\n'));
      const result = await fullScan();
      console.log(chalk.green(`✓ Scanned ${result.totalSessions} sessions (${result.newSessions} updated)`));
      break;
    }

    case 'dashboard':
    case 'd': {
      console.log(chalk.bold('🐕 CostDog — Cost & Resource Monitor\n'));

      // Scan first
      const scanResult = await fullScan();
      console.log(chalk.gray(`  Scanned ${scanResult.totalSessions} sessions\n`));

      const data = getDashboardData();

      printSection('Today', data.today);
      printSection('Last 7 Days', data.week);
      printSection('Last 30 Days', data.month);
      printSection('All Time', data.allTime);

      // Alerts
      if (data.alerts.length > 0) {
        console.log();
        console.log(chalk.bold.red('⚠  Alerts:'));
        for (const a of data.alerts) {
          const icon = a.level === 'danger' ? '🔴' : a.level === 'warn' ? '🟡' : '🔵';
          console.log(`  ${icon} ${a.message}`);
        }
      }

      // Recent sessions
      if (data.recentSessions.length > 0) {
        console.log();
        console.log(chalk.bold('Recent Sessions:'));
        console.log(chalk.gray('  ' + pad('Source', 12) + pad('Model', 22) + pad('Project', 18) + pad('Tokens', 12) + 'Cost'));
        console.log(chalk.gray('  ' + '─'.repeat(70)));
        for (const s of data.recentSessions.slice(0, 10)) {
          const tokens = (s as any).input_tokens + (s as any).output_tokens;
          const cost = (s as any).cost || 0;
          console.log(
            '  ' +
            pad((s as any).source, 12) +
            pad((s as any).model || '-', 22) +
            pad((s as any).project || '-', 18) +
            pad(formatTokens(tokens), 12) +
            formatCost(cost)
          );
        }
      }

      console.log();
      break;
    }

    case 'watch': {
      const intervalMs = (parseInt(args[1]) || 60) * 1000;
      console.log(chalk.bold(`🐕 CostDog — Watch mode (refresh every ${intervalMs/1000}s)\n`));
      console.log(chalk.gray('Press Ctrl+C to stop\n'));

      const refresh = async () => {
        // Clear screen
        process.stdout.write('\x1B[2J\x1B[0f');
        console.log(chalk.bold('🐕 CostDog — Cost & Resource Monitor'));
        console.log(chalk.gray(`  Last refresh: ${new Date().toLocaleTimeString()}\n`));

        await fullScan();
        const data = getDashboardData();

        printSection('Today', data.today);
        printSection('Last 7 Days', data.week);

        if (data.alerts.length > 0) {
          console.log();
          for (const a of data.alerts) {
            const icon = a.level === 'danger' ? '🔴' : a.level === 'warn' ? '🟡' : '🔵';
            console.log(`  ${icon} ${a.message}`);
          }
        }

        console.log();
        console.log(chalk.gray(`  Next refresh in ${intervalMs/1000}s...`));
      };

      await refresh();
      setInterval(refresh, intervalMs);
      break;
    }

    case 'pricing':
    case 'p': {
      console.log(chalk.bold('🐕 CostDog — Model Pricing\n'));
      const pricing = await loadPricing();
      console.log(chalk.gray(`  Loaded ${pricing.length} models from OpenRouter\n`));

      // Show pricing for models in use
      const modelsInUse = ['claude-opus-4-8', 'claude-opus-4-7', 'claude-sonnet-4-6', 'deepseek-v4-pro', 'gpt-5.4', 'glm-5.2', 'mimo-v2.5-pro'];
      console.log(chalk.bold('  Model Pricing (per 1M tokens):'));
      console.log(chalk.gray('  ' + pad('Model', 25) + pad('Input', 12) + pad('Output', 12) + 'Cache Read'));
      console.log(chalk.gray('  ' + '─'.repeat(55)));
      for (const m of modelsInUse) {
        const price = findModelPrice(m, pricing);
        if (price) {
          console.log(
            '  ' +
            pad(m, 25) +
            pad(formatCost(price.input), 12) +
            pad(formatCost(price.output), 12) +
            formatCost(price.input * 0.1)
          );
        } else {
          console.log('  ' + pad(m, 25) + chalk.gray('not found'));
        }
      }
      console.log();
      break;
    }

    case 'web':
    case 'w': {
      const { startWebServer } = await import('../web/server');
      const port = args[1] ? parseInt(args[1]) : undefined;
      startWebServer(port);
      break;
    }

    case 'desktop': {
      // Start web server + open browser mini-dashboard
      const { startWebServer } = await import('../web/server');
      const port = 3456;
      startWebServer(port);
      console.log(chalk.gray(`\n  Mini-dashboard: http://localhost:${port}\n`));
      console.log(chalk.bold('  Tip: Open in a small browser window and pin it to your desktop'));
      console.log(chalk.gray('  For native desktop widget, run: npm run tauri:build\n'));
      break;
    }

    case 'help':
    case 'h':
    default: {
      console.log(chalk.bold('🐕 CostDog') + chalk.gray(` v${VERSION} — Claude Code & Codex Cost Monitor\n`));
      console.log('Usage: costdog [command]\n');
      console.log('Commands:');
      console.log('  dashboard, d    Show cost dashboard (default)');
      console.log('  scan, s         Scan logs and update database');
      console.log('  watch [sec]     Auto-refresh dashboard (default 60s)');
      console.log('  pricing, p      Show model pricing from OpenRouter');
      console.log('  web [port]      Start web dashboard server');
      console.log('  desktop         Start web + show mini-dashboard URL');
      console.log('  help, h         Show this help');
      console.log();
      console.log('Environment:');
      console.log('  CODEX_HOME        Override Codex config directory');
      console.log('  COSTDOG_DATA_DIR  Override CostDog data directory');
      console.log('  COSTDOG_PORT      Web dashboard port (default 3456)');
      console.log();
    }
  }
}

main().catch(console.error);
