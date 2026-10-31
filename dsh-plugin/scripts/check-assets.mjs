#!/usr/bin/env node
/**
 * Verify that the checked-in DSH plugin assets are in sync with govctl's
 * `.claude/` asset SSOT. Thin wrapper around `sync-assets.mjs --check`.
 *
 * Exits 0 when up to date, 1 on drift (listing every drift line), 2 on error.
 * Never writes to disk.
 *
 * Usage: node dsh-plugin/scripts/check-assets.mjs
 */

import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const SYNC_SCRIPT = path.join(SCRIPT_DIR, 'sync-assets.mjs');

const result = spawnSync(process.execPath, [SYNC_SCRIPT, '--check'], { stdio: 'inherit' });

if (result.error) {
  console.error(`check-assets: error: failed to run sync-assets.mjs: ${result.error.message}`);
  process.exit(2);
}
if (typeof result.status !== 'number') {
  console.error('check-assets: error: sync-assets.mjs terminated by signal');
  process.exit(2);
}
process.exit(result.status);
