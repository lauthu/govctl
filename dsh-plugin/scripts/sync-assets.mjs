#!/usr/bin/env node
/**
 * Regenerate the DSH plugin's bundled skill assets from govctl's asset SSOT.
 *
 * Source of truth: <repo>/.claude/skills/<name>/SKILL.md and <repo>/.claude/agents/<name>.md
 * Output:          <repo>/dsh-plugin/assets/skills/<name>/SKILL.md
 *                  <repo>/dsh-plugin/assets/manifest.json
 *
 * Normalization (deterministic; no network, no timestamps):
 *   - frontmatter keeps only `name` and `description`, both double-quoted;
 *     `allowed-tools` and `argument-hint` are dropped (DSH does not use them);
 *   - `$ARGUMENTS` is replaced with the natural phrase "the task the user named"
 *     because DSH does not substitute Claude-style argument placeholders;
 *   - reviewer agent prompts become skills: the agent body is reused verbatim
 *     after one prepended instruction line, because DSH 0.2.1 has no
 *     Markdown-defined agent mechanism.
 *
 * Usage:
 *   node dsh-plugin/scripts/sync-assets.mjs           # write assets
 *   node dsh-plugin/scripts/sync-assets.mjs --check   # verify only, never writes
 *
 * Exit codes: 0 success / up to date, 1 drift (--check only), 2 internal error.
 */

import { createHash } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmdirSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_PATH = fileURLToPath(import.meta.url);
const SCRIPT_DIR = path.dirname(SCRIPT_PATH);
const REPO_ROOT = path.resolve(SCRIPT_DIR, '..', '..');
const PLUGIN_ROOT = path.resolve(SCRIPT_DIR, '..');
const CLAUDE_DIR = path.join(REPO_ROOT, '.claude');
const SKILLS_SRC_DIR = path.join(CLAUDE_DIR, 'skills');
const AGENTS_SRC_DIR = path.join(CLAUDE_DIR, 'agents');
const ASSETS_DIR = path.join(PLUGIN_ROOT, 'assets');
const SKILLS_OUT_DIR = path.join(ASSETS_DIR, 'skills');
const MANIFEST_PATH = path.join(ASSETS_DIR, 'manifest.json');
const CARGO_TOML_PATH = path.join(REPO_ROOT, 'Cargo.toml');

const ARGUMENTS_PLACEHOLDER = '`$ARGUMENTS`';
const ARGUMENTS_PHRASE = 'the task the user named';
const REVIEWER_INSTRUCTION =
  'Apply this review to the target the user names; when no target is named, review the current change set.';

const NAME_RE = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const FRONTMATTER_RE = /^---\n([\s\S]*?)\n---\n/;
const VERSION_RE = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

class SyncError extends Error {}

function fail(message) {
  throw new SyncError(message);
}

/** Read a UTF-8 text file, failing with a contextual message. */
function readText(filePath, label) {
  try {
    return readFileSync(filePath, 'utf8');
  } catch (error) {
    fail(`cannot read ${label} ${filePath}: ${error.message}`);
  }
}

/** Parse YAML frontmatter fields (single-line scalar values only). */
function parseFrontmatter(raw, sourceLabel) {
  const fields = new Map();
  for (const line of raw.split('\n')) {
    if (line.trim() === '') continue;
    const colon = line.indexOf(':');
    if (colon <= 0) {
      fail(`${sourceLabel}: unsupported frontmatter line ${JSON.stringify(line)}`);
    }
    const key = line.slice(0, colon).trim();
    let value = line.slice(colon + 1).trim();
    if (value.startsWith('"')) {
      try {
        value = JSON.parse(value);
      } catch (error) {
        fail(`${sourceLabel}: invalid double-quoted frontmatter value for ${key}: ${error.message}`);
      }
    } else if (value.startsWith("'")) {
      if (!value.endsWith("'") || value.length < 2) {
        fail(`${sourceLabel}: unterminated single-quoted frontmatter value for ${key}`);
      }
      value = value.slice(1, -1).replace(/''/g, "'");
    }
    if (fields.has(key)) fail(`${sourceLabel}: duplicate frontmatter key ${key}`);
    fields.set(key, value);
  }
  return fields;
}

/** Split a Markdown asset into frontmatter fields and the raw body after the closing fence. */
function splitMarkdownAsset(text, sourceLabel) {
  const match = FRONTMATTER_RE.exec(text);
  if (!match) {
    fail(`${sourceLabel}: missing or malformed frontmatter (expected leading "---" ... "---")`);
  }
  return {
    fields: parseFrontmatter(match[1], sourceLabel),
    body: text.slice(match[0].length),
  };
}

/** Require exactly one `name` and one `description`, and validate the name shape. */
function requireIdentity(fields, sourceLabel) {
  const name = fields.get('name');
  const description = fields.get('description');
  if (typeof name !== 'string' || name.trim() === '') {
    fail(`${sourceLabel}: missing frontmatter "name"`);
  }
  if (typeof description !== 'string' || description.trim() === '') {
    fail(`${sourceLabel}: missing frontmatter "description"`);
  }
  if (!NAME_RE.test(name)) {
    fail(`${sourceLabel}: frontmatter name ${JSON.stringify(name)} is not kebab-case`);
  }
  if (description.includes('\n')) {
    fail(`${sourceLabel}: frontmatter description must be a single line`);
  }
  return { name, description };
}

/** Replace Claude's `$ARGUMENTS` placeholder with natural prose, then assert none is left. */
function replaceArguments(body, sourceLabel) {
  let out = body;
  if (out.includes(ARGUMENTS_PLACEHOLDER)) {
    out = out.split(ARGUMENTS_PLACEHOLDER).join(ARGUMENTS_PHRASE);
  }
  if (out.includes('$ARGUMENTS')) {
    out = out.split('$ARGUMENTS').join(ARGUMENTS_PHRASE);
  }
  if (out.includes('$ARGUMENTS')) {
    fail(`${sourceLabel}: could not normalize $ARGUMENTS`);
  }
  return out;
}

/** Ensure a body starts after one blank line and ends with a newline. */
function normalizeBody(body) {
  let out = body;
  if (!out.startsWith('\n')) out = `\n${out}`;
  if (!out.endsWith('\n')) out += '\n';
  return out;
}

/** Build one normalized SKILL.md text with frontmatter containing only name + description. */
function renderSkillText(name, description, body) {
  return `---\nname: ${JSON.stringify(name)}\ndescription: ${JSON.stringify(
    description,
  )}\n---\n${body}`;
}

/** Parse a generated SKILL.md back and enforce the DSH skill contract. */
function assertNormalized(text, name, sourceLabel) {
  const { fields, body } = splitMarkdownAsset(text, `generated ${sourceLabel}`);
  const keys = [...fields.keys()];
  if (keys.length !== 2 || keys[0] !== 'name' || keys[1] !== 'description') {
    fail(`generated ${sourceLabel}: frontmatter must contain only name and description (got ${keys.join(', ')})`);
  }
  if (fields.get('name') !== name) {
    fail(`generated ${sourceLabel}: frontmatter name mismatch`);
  }
  if (body.includes('$ARGUMENTS')) {
    fail(`generated ${sourceLabel}: body still contains $ARGUMENTS`);
  }
  if (body.trim() === '') {
    fail(`generated ${sourceLabel}: empty body`);
  }
}

/** Extract `[package].version` from Cargo.toml. */
function readGovctlVersion() {
  const text = readText(CARGO_TOML_PATH, 'Cargo.toml');
  let inPackage = false;
  for (const line of text.split('\n')) {
    const section = /^\s*\[([^\]]+)\]\s*$/.exec(line);
    if (section) {
      inPackage = section[1].trim() === 'package';
      continue;
    }
    if (!inPackage) continue;
    const version = /^\s*version\s*=\s*"([^"]+)"\s*(?:#.*)?$/.exec(line);
    if (version) {
      const value = version[1];
      if (!VERSION_RE.test(value)) fail(`Cargo.toml [package].version ${JSON.stringify(value)} is not a semver version`);
      return value;
    }
  }
  fail('cannot find [package].version in Cargo.toml');
}

/** List skill source descriptors, sorted by output name. */
function discoverSkillSources() {
  let entries;
  try {
    entries = readdirSync(SKILLS_SRC_DIR, { withFileTypes: true });
  } catch (error) {
    fail(`cannot read ${SKILLS_SRC_DIR}: ${error.message}`);
  }
  return entries
    .filter((entry) => entry.isDirectory())
    .map((entry) => {
      const sourcePath = path.join(SKILLS_SRC_DIR, entry.name, 'SKILL.md');
      return {
        kind: 'skill',
        sourceRel: `.claude/skills/${entry.name}/SKILL.md`,
        sourcePath,
        dirName: entry.name,
      };
    });
}

/** List reviewer agent source descriptors, sorted by file name. */
function discoverAgentSources() {
  let entries;
  try {
    entries = readdirSync(AGENTS_SRC_DIR, { withFileTypes: true });
  } catch (error) {
    fail(`cannot read ${AGENTS_SRC_DIR}: ${error.message}`);
  }
  return entries
    .filter((entry) => entry.isFile() && entry.name.endsWith('.md'))
    .map((entry) => ({
      kind: 'agent',
      sourceRel: `.claude/agents/${entry.name}`,
      sourcePath: path.join(AGENTS_SRC_DIR, entry.name),
      baseName: entry.name.slice(0, -'.md'.length),
    }));
}

/** Build the complete expected asset tree in memory. */
function buildAssets() {
  const govctlVersion = readGovctlVersion();
  const sources = [
    ...discoverSkillSources().map((source) => ({ ...source, sortKey: source.dirName })),
    ...discoverAgentSources().map((source) => ({ ...source, sortKey: source.baseName })),
  ].sort((a, b) => (a.sourceRel < b.sourceRel ? -1 : a.sourceRel > b.sourceRel ? 1 : 0));

  const byName = new Map();
  const files = new Map();

  for (const source of sources) {
    const text = readText(source.sourcePath, source.sourceRel);
    const { fields, body } = splitMarkdownAsset(text, source.sourceRel);
    const { name, description } = requireIdentity(fields, source.sourceRel);

    const expectedName = source.kind === 'skill' ? source.dirName : source.baseName;
    if (name !== expectedName) {
      fail(`${source.sourceRel}: frontmatter name ${JSON.stringify(name)} does not match source path name ${JSON.stringify(expectedName)}`);
    }
    if (byName.has(name)) {
      fail(`${source.sourceRel}: duplicate output skill name ${JSON.stringify(name)} (first from ${byName.get(name).sourceRel})`);
    }

    let normalizedBody;
    if (source.kind === 'skill') {
      normalizedBody = normalizeBody(replaceArguments(body, source.sourceRel));
    } else {
      // Reviewer agents become skills: prepend one instruction line, keep the prompt body intact.
      normalizedBody = normalizeBody(`\n${REVIEWER_INSTRUCTION}\n${body}`);
    }

    const output = renderSkillText(name, description, normalizedBody);
    assertNormalized(output, name, source.sourceRel);

    const relPath = `${name}/SKILL.md`;
    files.set(relPath, {
      name,
      sourceRel: source.sourceRel,
      content: Buffer.from(output, 'utf8'),
    });
    byName.set(name, source);
  }

  const skills = [...files.values()]
    .map((file) => ({
      name: file.name,
      source: file.sourceRel,
      sha256: createHash('sha256').update(file.content).digest('hex'),
    }))
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));

  const manifestText = `${JSON.stringify(
    { govctlVersion, source: '.claude', skills },
    null,
    2,
  )}\n`;

  return {
    govctlVersion,
    files,
    manifestText,
    counts: {
      total: skills.length,
      ported: sources.filter((source) => source.kind === 'skill').length,
      reviewers: sources.filter((source) => source.kind === 'agent').length,
    },
  };
}

/** Recursively list files under dir, returning forward-slash paths relative to dir. */
function listFilesRelative(dir) {
  if (!existsSync(dir)) return [];
  const out = [];
  const walk = (current, relBase) => {
    const entries = readdirSync(current, { withFileTypes: true }).sort((a, b) =>
      a.name < b.name ? -1 : a.name > b.name ? 1 : 0,
    );
    for (const entry of entries) {
      const absolute = path.join(current, entry.name);
      const relative = relBase === '' ? entry.name : `${relBase}/${entry.name}`;
      if (entry.isDirectory()) walk(absolute, relative);
      else if (entry.isFile()) out.push(relative);
    }
  };
  walk(dir, '');
  return out;
}

/** Prune now-empty directories under dir (never removes dir itself). */
function pruneEmptyDirs(dir) {
  if (!existsSync(dir)) return;
  const entries = readdirSync(dir, { withFileTypes: true }).sort((a, b) =>
    a.name < b.name ? -1 : a.name > b.name ? 1 : 0,
  );
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const absolute = path.join(dir, entry.name);
    pruneEmptyDirs(absolute);
    if (readdirSync(absolute).length === 0) rmdirSync(absolute);
  }
}

/** Compare expected in-memory assets with what is on disk; return a sorted drift list. */
function collectDrift(assets) {
  const drift = [];
  for (const [relPath, file] of assets.files) {
    const absolute = path.join(SKILLS_OUT_DIR, relPath);
    if (!existsSync(absolute)) {
      drift.push(`assets/skills/${relPath}: missing`);
      continue;
    }
    const actual = readFileSync(absolute);
    if (!actual.equals(file.content)) drift.push(`assets/skills/${relPath}: content differs`);
  }
  if (!existsSync(MANIFEST_PATH)) drift.push('assets/manifest.json: missing');
  else if (readFileSync(MANIFEST_PATH, 'utf8') !== assets.manifestText) {
    drift.push('assets/manifest.json: content differs');
  }
  const expected = new Set([...assets.files.keys()]);
  for (const relPath of listFilesRelative(SKILLS_OUT_DIR)) {
    if (!expected.has(relPath)) drift.push(`assets/skills/${relPath}: unexpected (not generated from .claude)`);
  }
  return drift.sort();
}

/** Write expected assets, remove stale generated files, and report what changed. */
function writeAssets(assets) {
  mkdirSync(SKILLS_OUT_DIR, { recursive: true });
  let written = 0;
  let unchanged = 0;
  for (const [relPath, file] of assets.files) {
    const absolute = path.join(SKILLS_OUT_DIR, relPath);
    mkdirSync(path.dirname(absolute), { recursive: true });
    if (existsSync(absolute) && readFileSync(absolute).equals(file.content)) {
      unchanged += 1;
      continue;
    }
    writeFileSync(absolute, file.content);
    written += 1;
  }

  let removed = 0;
  const expected = new Set([...assets.files.keys()]);
  for (const relPath of listFilesRelative(SKILLS_OUT_DIR)) {
    if (expected.has(relPath)) continue;
    unlinkSync(path.join(SKILLS_OUT_DIR, relPath));
    removed += 1;
  }
  pruneEmptyDirs(SKILLS_OUT_DIR);

  let manifestAction = 'unchanged';
  if (!existsSync(MANIFEST_PATH) || readFileSync(MANIFEST_PATH, 'utf8') !== assets.manifestText) {
    writeFileSync(MANIFEST_PATH, assets.manifestText);
    manifestAction = 'written';
  }

  const { total, ported, reviewers } = assets.counts;
  console.log(
    `sync-assets: ${total} skills (${ported} ported, ${reviewers} reviewers), govctl ${assets.govctlVersion}; ` +
      `files written=${written} unchanged=${unchanged} removed=${removed}; manifest=${manifestAction}`,
  );
  return { written, unchanged, removed };
}

function runCheck(assets) {
  const drift = collectDrift(assets);
  if (drift.length > 0) {
    for (const line of drift) console.error(`drift: ${line}`);
    console.error(
      `check-assets: ${drift.length} drift(s) between .claude sources and dsh-plugin/assets; ` +
        'run "node dsh-plugin/scripts/sync-assets.mjs" to regenerate',
    );
    return 1;
  }
  const { total, ported, reviewers } = assets.counts;
  console.log(
    `check-assets: ${total} skills (${ported} ported, ${reviewers} reviewers) and manifest up to date; ` +
      `govctl ${assets.govctlVersion}`,
  );
  return 0;
}

function main(argv) {
  const args = argv.slice(2);
  const known = new Set(['--check']);
  for (const arg of args) {
    if (!known.has(arg)) {
      console.error(`sync-assets: unknown argument ${JSON.stringify(arg)}`);
      console.error('usage: node dsh-plugin/scripts/sync-assets.mjs [--check]');
      return 2;
    }
  }
  const checkOnly = args.includes('--check');
  const assets = buildAssets();
  return checkOnly ? runCheck(assets) : (writeAssets(assets), 0);
}

try {
  process.exit(main(process.argv));
} catch (error) {
  if (error instanceof SyncError) {
    console.error(`sync-assets: error: ${error.message}`);
  } else {
    console.error(`sync-assets: internal error: ${error instanceof Error ? error.stack : String(error)}`);
  }
  process.exit(2);
}
