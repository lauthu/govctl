/**
 * Package contract: DSH bundle manifest, root-row patch, engine admission and
 * npm publish contents.
 */
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const PACKAGE_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..')
const packageJson = JSON.parse(readFileSync(path.join(PACKAGE_ROOT, 'package.json'), 'utf8'))
const patch = readFileSync(path.join(PACKAGE_ROOT, 'cordis.patch.yml'), 'utf8')
const readme = readFileSync(path.join(PACKAGE_ROOT, 'README.md'), 'utf8')

function npmAvailable() {
  try {
    execFileSync('npm', ['--version'], { stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}

test('package identity and ESM entry points', () => {
  assert.equal(packageJson.name, 'govctl-dsh')
  assert.match(packageJson.version, /^\d+\.\d+\.\d+$/)
  assert.equal(packageJson.type, 'module')
  assert.equal(packageJson.main, './lib/index.js')
  assert.equal(packageJson.exports['.'], './lib/index.js')
  assert.equal(packageJson.exports['./cordis.patch.yml'], './cordis.patch.yml')
  assert.equal(packageJson.exports['./package.json'], './package.json')
  assert.equal(packageJson.license, 'MIT')
  assert.equal(packageJson.scripts.test, 'node --test')
})

test('DSH bundle manifest declares manifestVersion and the root patch', () => {
  assert.equal(packageJson.dsh.manifestVersion, 1)
  assert.equal(packageJson.dsh.bundle.patch, './cordis.patch.yml')
  // The patch file must exist at the declared relative path.
  assert.ok(readFileSync(path.join(PACKAGE_ROOT, 'cordis.patch.yml'), 'utf8').length > 0)
})

test('engine admission admits the installed DSH runtime and bounds the next minor', () => {
  assert.equal(packageJson.engines.dsh, '>=0.2.1-rc.5 <0.3.0-0')
  assert.equal(packageJson.engines.node, '>=20')
})

test('the llm peer is optional so the plugin works without it', () => {
  assert.equal(typeof packageJson.peerDependencies['@deepseek-ai/dsh-llm'], 'string')
  assert.equal(packageJson.peerDependenciesMeta['@deepseek-ai/dsh-llm'].optional, true)
  // No hard runtime dependencies at all: the package only imports node builtins
  // and optionally the peer.
  assert.equal(packageJson.dependencies, undefined)
})

test('cordis.patch.yml inserts one root row named govctl-dsh with the documented config', () => {
  assert.match(patch, /^- insert:/m)
  assert.match(patch, /id: govctl\b/)
  assert.match(patch, /name: govctl-dsh\b/)
  for (const key of [
    'binaryPath: govctl',
    'toolTimeoutMs: 20000',
    'maxOutputBytes: 65536',
    'enablePolicySection: true',
    'enableSessionContext: true',
    'enableEditAdvisory: true',
  ]) {
    assert.ok(patch.includes(key), `patch must carry ${key}`)
  }
  // The row must not disable the plugin.
  assert.ok(!/disabled:\s*true/.test(patch), 'govctl-dsh row must be enabled')
})

test('files whitelist publishes runtime, assets and docs but not tests/scripts/source docs', () => {
  assert.deepEqual(packageJson.files, ['lib/', 'assets/', 'cordis.patch.yml', 'README.md', 'LICENSE'])
  for (const excluded of ['test', 'scripts', 'docs']) {
    assert.ok(!packageJson.files.some((entry) => entry.startsWith(excluded)), `${excluded} must not be published`)
  }
})

test('the test script uses portable default discovery (Node 20/22/24)', () => {
  // Node 22+ treats positional --test arguments as glob patterns, so a bare
  // directory argument ("node --test test/") fails with MODULE_NOT_FOUND. Default
  // discovery (`node --test` with no args) works on Node 20, 22 and 24.
  assert.equal(packageJson.scripts.test, 'node --test')
})

test('README exists and documents the shipped surface', () => {
  assert.ok(readme.length > 2_000, 'README should be substantive')
  for (const toolName of [
    'govctl_describe',
    'govctl_status',
    'govctl_list',
    'govctl_get',
    'govctl_show',
    'govctl_search',
    'govctl_check',
    'govctl_verify',
    'govctl_trace',
    'govctl_work_new',
    'govctl_work_move',
    'govctl_edit',
    'govctl_render',
    'govctl_run',
  ]) {
    assert.ok(readme.includes(toolName), `README must document ${toolName}`)
  }
  assert.ok(readme.includes('/gov-status'))
  assert.ok(readme.includes('/gov-check'))
  // Install/verify workflow required by the delivery plan.
  assert.ok(/node --test|npm test/.test(readme), 'README must document how to run the tests')
  assert.ok(readme.includes('validate-with-dsh.sh'), 'README must reference the DSH end-to-end script')
})

test('npm pack publishes only the intended files', { skip: !npmAvailable() }, () => {
  const raw = execFileSync('npm', ['pack', '--dry-run', '--json'], { cwd: PACKAGE_ROOT, encoding: 'utf8' })
  const [manifest] = JSON.parse(raw)
  assert.equal(manifest.name, 'govctl-dsh')
  const files = manifest.files.map((entry) => entry.path)
  for (const required of ['LICENSE', 'README.md', 'assets/manifest.json', 'cordis.patch.yml', 'package.json', 'lib/index.js']) {
    assert.ok(files.includes(required), `packed tarball missing ${required}`)
  }
  assert.equal(files.filter((file) => file.startsWith('lib/')).length, 8, 'all 8 lib modules must ship')
  assert.equal(files.filter((file) => /^assets\/skills\/[^/]+\/SKILL\.md$/.test(file)).length, 17)
  for (const forbidden of ['test/', 'scripts/', 'docs/']) {
    assert.ok(!files.some((file) => file.startsWith(forbidden)), `packed tarball must not contain ${forbidden}`)
  }
})
