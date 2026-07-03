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

// ZCode CLI stores its data in ~/.zcode. Respects ZCODE_HOME if set.
export function getZcodeDir(): string {
  return process.env.ZCODE_HOME || path.join(os.homedir(), '.zcode');
}

export function getZcodeDbPath(): string {
  return path.join(getZcodeDir(), 'cli', 'db', 'db.sqlite');
}

// OpenCode stores its DB at ~/.local/share/opencode/opencode.db.
// OPENCODE_DB may be an absolute path to the db file itself; XDG_DATA_HOME overrides the base dir.
export function getOpencodeDataDir(): string {
  const opencodeDb = process.env.OPENCODE_DB;
  if (opencodeDb && path.isAbsolute(opencodeDb)) {
    return path.dirname(opencodeDb);
  }
  const xdg = process.env.XDG_DATA_HOME;
  if (xdg) return path.join(xdg, 'opencode');
  return path.join(os.homedir(), '.local', 'share', 'opencode');
}

export function getOpencodeDbPath(): string {
  const opencodeDb = process.env.OPENCODE_DB;
  if (opencodeDb && path.isAbsolute(opencodeDb)) return opencodeDb;
  return path.join(getOpencodeDataDir(), 'opencode.db');
}

export function getCostDogDbPath(): string {
  const dataDir = process.env.COSTDOG_DATA_DIR || path.join(os.homedir(), '.costdog');
  return path.join(dataDir, 'costdog.sqlite');
}

export function getCostDogConfigDir(): string {
  return process.env.COSTDOG_DATA_DIR || path.join(os.homedir(), '.costdog');
}
