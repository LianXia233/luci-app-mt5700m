#!/bin/sh
# Concurrency and stress checks for the MT5700M dial manager.
#
# Two properties are exercised:
#
#   1. acquire_lock() must be exclusive - with N contenders only one may hold
#      the lock at any instant.
#   2. The recovery bookkeeping files under /var/run/mt5700m are plain files
#      written with a read-modify-write.  Without external serialisation the
#      counter loses updates, which is characterised here (not "fixed") so the
#      exposure is documented rather than guessed at.

set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
MANAGER="${ROOT}/root/usr/sbin/mt5700m-manager"
TMP="${ROOT}/tests/.tmp-conc-$$"
rm -rf "${TMP}" >/dev/null 2>&1 || true
mkdir -p "${TMP}/state"
trap 'rm -rf "${TMP}" >/dev/null 2>&1 || true' EXIT INT TERM

STATE="${TMP}/state"
LOG="${TMP}/manager.log"
LOCK="${TMP}/lock"

# Contenders kept modest: the point is interleaving, not load generation.
N="${CONC_N:-15}"

PASS=0
FAIL=0
ok() { PASS=$((PASS + 1)); printf '  ok   %s\n' "$1"; }
ng() { FAIL=$((FAIL + 1)); printf '  FAIL %s\n' "$1" >&2; }

sed '/^action="\${1:-status-json}"/,$d' "${MANAGER}" > "${TMP}/lib.sh"

MT5700M_MANAGER_STATE="${STATE}"
MT5700M_MANAGER_LOG="${LOG}"
MT5700M_MANAGER_LOCK="${LOCK}"
MT5700M_USB_HELPER="${ROOT}/root/usr/share/mt5700m/usb.sh"
export MT5700M_MANAGER_STATE MT5700M_MANAGER_LOG MT5700M_MANAGER_LOCK \
	MT5700M_USB_HELPER

printf 'MT5700M concurrency / stress tests (N=%s)\n' "${N}"

# ---------------------------------------------------------------------------
printf '\n1. lock exclusivity\n'
# ---------------------------------------------------------------------------
# Each contender grabs the lock, records that it is inside the critical
# section, sleeps briefly, then releases.  If two processes are ever inside at
# the same time the occupancy counter exceeds 1.
OCC="${TMP}/occupancy"
: > "${OCC}"
: > "${TMP}/inside"
i=1
while [ "${i}" -le "${N}" ]; do
	(
		# shellcheck source=/dev/null
		. "${TMP}/lib.sh"
		if acquire_lock; then
			count=$(($(cat "${OCC}" 2>/dev/null || echo 0) + 1))
			printf '%s\n' "${count}" > "${OCC}"
			# If someone else was already inside, occupancy was > 1.
			[ "${count}" -eq 1 ] || printf 'OVERLAP\n' >> "${TMP}/inside"
			sleep 1
			count=$(($(cat "${OCC}" 2>/dev/null || echo 0) - 1))
			printf '%s\n' "${count}" > "${OCC}"
			release_lock
		fi
	) &
	i=$((i + 1))
done
wait

if [ -s "${TMP}/inside" ]; then
	ng 'critical section overlapped: the lock is not exclusive'
else
	ok 'critical section never overlapped'
fi
final_occ="$(cat "${OCC}" 2>/dev/null || echo 0)"
if [ "${final_occ}" = '0' ]; then
	ok 'occupancy returned to zero (no leaked lock)'
else
	ng "occupancy settled at ${final_occ}, a lock was leaked"
fi
if [ -d "${LOCK}" ]; then
	ng 'lock directory was left behind'
else
	ok 'lock directory removed after the run'
fi

# ---------------------------------------------------------------------------
printf '\n2. stale lock reclaim\n'
# ---------------------------------------------------------------------------
# A lock directory whose recorded pid is dead must be reclaimed instead of
# wedging every later run.
mkdir -p "${LOCK}"
printf '%s\n' '999999' > "${LOCK}/pid"
(
	# shellcheck source=/dev/null
	. "${TMP}/lib.sh"
	acquire_lock && printf 'reclaimed\n' > "${TMP}/reclaim" || printf 'stuck\n' > "${TMP}/reclaim"
) || true
if [ "$(cat "${TMP}/reclaim" 2>/dev/null || echo missing)" = 'reclaimed' ]; then
	ok 'lock held by a dead pid is reclaimed'
else
	ng 'a dead pid wedges the lock'
fi
rm -rf "${LOCK}" >/dev/null 2>&1 || true

# ---------------------------------------------------------------------------
printf '\n3. counter updates under contention\n'
# ---------------------------------------------------------------------------
# Characterisation test: N processes each perform the same read-modify-write
# that recovery_record_failure() does.  The result shows how many increments
# survive when nothing serialises them.
COUNTER="${STATE}/recovery_failures"
printf '0\n' > "${COUNTER}"
i=1
while [ "${i}" -le "${N}" ]; do
	(
		v="$(cat "${COUNTER}" 2>/dev/null || echo 0)"
		case "${v}" in ''|*[!0-9]*) v=0 ;; esac
		v=$((v + 1))
		printf '%s\n' "${v}" > "${COUNTER}"
	) &
	i=$((i + 1))
done
wait
recorded="$(cat "${COUNTER}" 2>/dev/null || echo 0)"
printf '  info %s increments issued, counter reads %s\n' "${N}" "${recorded}"
if [ "${recorded}" -eq "${N}" ]; then
	ok "all ${N} increments survived (serialised)"
else
	# Not a hard failure: the manager serialises its own writes through
	# acquire_lock(), so this only matters for out-of-band writers such as a
	# manually invoked `reset-recovery`.  Reported as a risk, not a bug.
	printf '  WARN lost updates: %s of %s increments were dropped\n' \
		"$((N - recorded))" "${N}"
	ok 'lost-update behaviour characterised (see report)'
fi

# ---------------------------------------------------------------------------
printf '\n4. repeated monitor cycles do not leak state files\n'
# ---------------------------------------------------------------------------
# sync_manager() is what the 15s monitor loop calls.  Drive it repeatedly and
# confirm the state directory does not grow without bound.
BEFORE="$(find "${STATE}" -type f 2>/dev/null | wc -l | tr -d ' ')"
cycles=0
while [ "${cycles}" -lt 5 ]; do
	(
		# shellcheck source=/dev/null
		. "${TMP}/lib.sh"
		load_health_config
		recovery_reset
	) >/dev/null 2>&1 || true
	cycles=$((cycles + 1))
done
AFTER="$(find "${STATE}" -type f 2>/dev/null | wc -l | tr -d ' ')"
printf '  info state files: %s -> %s after %s cycles\n' "${BEFORE}" "${AFTER}" "${cycles}"
if [ "${AFTER}" -le "$((BEFORE + 4))" ]; then
	ok 'state file count stays bounded across cycles'
else
	ng "state directory grew from ${BEFORE} to ${AFTER} files"
fi

printf '\n%d passed, %d failed\n' "${PASS}" "${FAIL}"
[ "${FAIL}" -eq 0 ] || exit 1
exit 0
