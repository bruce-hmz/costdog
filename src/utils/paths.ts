import * as path from 'path';
import * as os from 'os';

/** Cross-platform home directory detection */
export function getClaudeCodeDir(): string {
  return path.join(os.homedir(), '.claude');
}

export function getCodexDir(): string {
  // Codex respects CODEX_HOME env var
  const codexHome = process.env.CODEX_HOME;
  if (codexHome) return codexHome;
  return path.join(os.homedir(), '.codex');
}

export function getClaudeSessionsDir(): string {
  return path.join(getClaudeCodeDir(), 'projects');
}

export function getCodexSessionsDir(): string {
  return path.join(getCodexDir(), 'sessions');
}

export function getCostDogDbPath(): string {
  const dataDir = process.env.COSTDOG_DATA_DIR || path.join(os.homedir(), '.costdog');
  return path.join(dataDir, 'costdog.sqlite');
}

export function getCostDogConfigDir(): string {
  return process.env.COSTDOG_DATA_DIR || path.join(os.homedir(), '.costdog');
}
