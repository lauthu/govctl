#!/usr/bin/env bash
#
# validate-with-dsh.sh — end-to-end validation of the govctl-dsh bundle against
# an installed DeepSeek Harness (DSH) runtime.
#
# What it does, using ONLY a scratch DSH_HOME (the real ~/.dsh is never touched):
#   1. initializes a scratch web-profile home,
#   2. installs this package (`dsh plugin --profile web add <package dir>`),
#   3. asserts the composed `--dump-config` contains the govctl-dsh bundle layer
#      and the root `govctl` row with the documented config,
#   4. boots `dsh web --no-open --port <free>` in the background and waits for the
#      plugin's load banner and bundled-skill count in the boot output,
#   5. terminates the server and removes the scratch home.
#
# Exit codes: 0 printed PASS; non-zero with a "FAIL:" line on any assertion.
#
# Environment overrides:
#   DSH_CLI                   path to the bundled DSH CLI (default: the installed app)
#   DSH_VALIDATE_PROFILE      scratch profile name inside the temporary DSH_HOME (default: web)
#   DSH_VALIDATE_TIMEOUT      seconds to wait for the plugin boot log (default: 90)
#
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PKG_DIR=$(cd "${SCRIPT_DIR}/.." && pwd)
DSH_CLI=${DSH_CLI:-/Applications/DeepSeek Harness.app/Contents/Resources/runtime/cli/bin/dsh}
# NOTE: do not reuse DSH_* names that the running harness exports (DSH_PROFILE is
# the live profile name, e.g. "desktop"); this script always uses a scratch home.
PROFILE=${DSH_VALIDATE_PROFILE:-web}
BOOT_TIMEOUT_SECONDS=${DSH_VALIDATE_TIMEOUT:-90}
EXPECTED_SKILLS=17

SERVER_PID=""
TMP_HOME=""

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

note() {
  printf '==> %s\n' "$*"
}

# Print the tail of a captured log to make a failure actionable.
dump_log() {
  local file=$1
  if [[ -f "$file" ]]; then
    printf -- '---- %s (tail) ----\n' "$file" >&2
    tail -n 40 "$file" >&2 || true
    printf -- '----------------------\n' >&2
  fi
}

cleanup() {
  local status=$?
  if [[ -n "${SERVER_PID}" ]] && kill -0 "${SERVER_PID}" 2>/dev/null; then
    kill -TERM "${SERVER_PID}" 2>/dev/null || true
    for _ in $(seq 1 50); do
      if ! kill -0 "${SERVER_PID}" 2>/dev/null; then
        break
      fi
      sleep 0.1
    done
    kill -KILL "${SERVER_PID}" 2>/dev/null || true
    wait "${SERVER_PID}" 2>/dev/null || true
  fi
  if [[ -n "${TMP_HOME}" && -d "${TMP_HOME}" ]]; then
    rm -rf "${TMP_HOME}"
  fi
  exit "${status}"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# ---------------------------------------------------------------------------
# Preconditions
# ---------------------------------------------------------------------------
[[ -x "${DSH_CLI}" ]] || fail "DSH CLI not found or not executable: ${DSH_CLI} (set DSH_CLI=...)"
[[ -f "${PKG_DIR}/package.json" ]] || fail "package.json not found in ${PKG_DIR}"
[[ -f "${PKG_DIR}/cordis.patch.yml" ]] || fail "cordis.patch.yml not found in ${PKG_DIR}"
command -v node >/dev/null 2>&1 || fail "node is required (free-port selection)"

# ---------------------------------------------------------------------------
# Scratch home (created before any dsh invocation so the real home is never touched)
# ---------------------------------------------------------------------------
TMP_HOME=$(mktemp -d "${TMPDIR:-/tmp}/dsh-govctl-validate.XXXXXX")
export DSH_HOME="${TMP_HOME}"
# The invoking harness session exports live-profile facts; scrub them so the
# scratch profile is fully isolated from ~/.dsh/profiles/desktop.
unset DSH_PROFILE DSH_PROFILE_DIR DSH_WEB_URL DSH_SESSION_ID
note "scratch DSH_HOME: ${DSH_HOME} (real ~/.dsh is untouched)"

DSH_VERSION=$("${DSH_CLI}" --version 2>&1) || fail "dsh --version failed: ${DSH_VERSION}"
note "DSH CLI: ${DSH_CLI} (${DSH_VERSION})"
note "package: ${PKG_DIR}"

note "initializing profile '${PROFILE}' from the shipped web template"
"${DSH_CLI}" --profile "${PROFILE}" --dump-config >"${TMP_HOME}/dump-initial.yml" 2>"${TMP_HOME}/init.err" ||
  {
    dump_log "${TMP_HOME}/init.err"
    fail "could not initialize profile '${PROFILE}' in a scratch DSH_HOME"
  }
[[ -d "${TMP_HOME}/profiles/${PROFILE}" ]] || fail "profile directory was not created: profiles/${PROFILE}"

# ---------------------------------------------------------------------------
# Install the local package into the scratch profile
# ---------------------------------------------------------------------------
note "installing the package into profile '${PROFILE}'"
"${DSH_CLI}" plugin --profile "${PROFILE}" add "${PKG_DIR}" >"${TMP_HOME}/plugin-add.log" 2>&1 ||
  {
    dump_log "${TMP_HOME}/plugin-add.log"
    fail "dsh plugin add failed"
  }
grep -q 'govctl-dsh' "${TMP_HOME}/plugin-add.log" || {
  dump_log "${TMP_HOME}/plugin-add.log"
  fail "plugin add log does not mention govctl-dsh"
}
note "installed govctl-dsh"

# ---------------------------------------------------------------------------
# Composed configuration assertions
# ---------------------------------------------------------------------------
note "checking the composed --dump-config"
"${DSH_CLI}" --profile "${PROFILE}" --dump-config >"${TMP_HOME}/dump.yml" 2>"${TMP_HOME}/dump.err" ||
  {
    dump_log "${TMP_HOME}/dump.err"
    fail "--dump-config failed after install"
  }

grep -qF '# == govctl-dsh' "${TMP_HOME}/dump.yml" ||
  fail "composed config is missing the '# == govctl-dsh' bundle layer"
grep -qE '^- id: govctl$' "${TMP_HOME}/dump.yml" || fail "composed config is missing the root row '- id: govctl'"
grep -qE '^  name: govctl-dsh$' "${TMP_HOME}/dump.yml" || fail "composed config row is missing 'name: govctl-dsh'"

layer=$(awk '/^# == govctl-dsh$/{flag=1; next} flag && /^# == /{exit} flag' "${TMP_HOME}/dump.yml")
for expected in 'binaryPath: govctl' 'toolTimeoutMs: 20000' 'maxOutputBytes: 65536' 'enablePolicySection: true'; do
  printf '%s\n' "${layer}" | grep -qF "${expected}" ||
    fail "govctl-dsh layer is missing the documented config '${expected}'"
done
if printf '%s\n' "${layer}" | grep -qE 'disabled: true'; then
  fail "the govctl row is disabled in the composed config"
fi
note "bundle layer and root row compose correctly"

# ---------------------------------------------------------------------------
# Live boot: load banner + bundled skills
# ---------------------------------------------------------------------------
PORT=$(node -e '
  const net = require("node:net");
  const server = net.createServer();
  server.listen(0, "127.0.0.1", () => {
    process.stdout.write(String(server.address().port));
    server.close();
  });
') || fail "could not allocate a free local port"
[[ -n "${PORT}" ]] || fail "could not allocate a free local port"

note "booting dsh web --no-open --port ${PORT} in the background"
"${DSH_CLI}" web --no-open --port "${PORT}" >"${TMP_HOME}/server.out" 2>"${TMP_HOME}/server.err" &
SERVER_PID=$!
note "dsh web pid: ${SERVER_PID}"

server_out="${TMP_HOME}/server.out"
deadline=$((SECONDS + BOOT_TIMEOUT_SECONDS))
wait_for_log() {
  local pattern=$1
  local what=$2
  while ! grep -qF "${pattern}" "${server_out}" 2>/dev/null; do
    if ! kill -0 "${SERVER_PID}" 2>/dev/null; then
      dump_log "${TMP_HOME}/server.out"
      dump_log "${TMP_HOME}/server.err"
      fail "dsh web exited before ${what}"
    fi
    if ((SECONDS >= deadline)); then
      dump_log "${TMP_HOME}/server.out"
      dump_log "${TMP_HOME}/server.err"
      fail "timed out after ${BOOT_TIMEOUT_SECONDS}s waiting for ${what} (pattern: ${pattern})"
    fi
    sleep 0.2
  done
}

wait_for_log '[govctl-dsh] loaded' "the govctl-dsh load banner"
wait_for_log "registered ${EXPECTED_SKILLS} bundled skills" "the ${EXPECTED_SKILLS}-skill registration banner"
if ! kill -0 "${SERVER_PID}" 2>/dev/null; then
  dump_log "${TMP_HOME}/server.out"
  dump_log "${TMP_HOME}/server.err"
  fail "dsh web exited right after loading the plugin"
fi
wait_for_log 'dsh web: http' "the dsh web listening URL"

note "observed boot lines:"
grep -F -e '[govctl-dsh]' -e 'dsh web: http://127.0.0.1:' "${server_out}" | sed 's/^/    /'

# ---------------------------------------------------------------------------
# Success
# ---------------------------------------------------------------------------
note "terminating dsh web (pid ${SERVER_PID})"
kill -TERM "${SERVER_PID}" 2>/dev/null || true
for _ in $(seq 1 50); do
  if ! kill -0 "${SERVER_PID}" 2>/dev/null; then
    break
  fi
  sleep 0.1
done
if kill -0 "${SERVER_PID}" 2>/dev/null; then
  kill -KILL "${SERVER_PID}" 2>/dev/null || true
fi
wait "${SERVER_PID}" 2>/dev/null || true
SERVER_PID=""

note "PASS: govctl-dsh composes, loads and registers its surface in a scratch DSH web profile"
exit 0
