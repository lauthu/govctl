/** Path predicates shared by the prompt, context and advisory code. */
import { existsSync } from 'node:fs'
import path from 'node:path'

/** `gov/` subdirectories whose artifacts are lifecycle-managed by govctl. */
const MANAGED_GOV_DIRS = new Set(['rfc', 'adr', 'work', 'guard', 'conformance'])

/** How many ancestor directories to inspect for a governed project root. */
const MAX_ANCESTOR_WALK = 40

/**
 * Find the nearest ancestor (including `cwd`) that contains `gov/config.toml`,
 * matching govctl's own upward project discovery. Returns undefined outside a
 * governed tree.
 */
export function findGovernedRoot(cwd) {
  if (typeof cwd !== 'string' || cwd.length === 0) return undefined
  let current = path.resolve(cwd)
  for (let depth = 0; depth < MAX_ANCESTOR_WALK; depth += 1) {
    try {
      if (existsSync(path.join(current, 'gov', 'config.toml'))) return current
    } catch {
      return undefined
    }
    const parent = path.dirname(current)
    if (parent === current) return undefined
    current = parent
  }
  return undefined
}

/** True when the workspace (or one of its ancestors) is a govctl-governed project. */
export function isGovernedProject(cwd) {
  return findGovernedRoot(cwd) !== undefined
}

/**
 * Return the project-relative path of a lifecycle-managed `gov/` artifact when
 * an `edit` / `write` call targets one; otherwise undefined.
 *
 * `filePath` is interpreted relative to the session working directory `cwd`,
 * then compared against the governed project root.
 */
export function managedEditTarget(cwd, exec, governedRoot = cwd) {
  if (exec?.name !== 'edit' && exec?.name !== 'write') return undefined
  const filePath = exec.arguments?.file_path
  if (typeof filePath !== 'string' || typeof cwd !== 'string' || cwd.length === 0) return undefined
  if (typeof governedRoot !== 'string' || governedRoot.length === 0) return undefined
  const absolute = path.isAbsolute(filePath) ? filePath : path.join(cwd, filePath)
  const relative = path.relative(governedRoot, absolute)
  if (relative.length === 0 || relative.startsWith('..') || path.isAbsolute(relative)) return undefined
  const [first, second, third] = relative.split(path.sep)
  if (first !== 'gov') return undefined
  if (second === 'releases.toml') return third === undefined ? relative : undefined
  return MANAGED_GOV_DIRS.has(second) ? relative : undefined
}
