#!/bin/sh
# DNS apply/verification tests.
#
# The script is exercised against a fake `uci` store and fake `ubus`/`logger`
# so the assertions are about the real bytes the boot path would write:
#   * both managed interfaces get the configured resolvers;
#   * the Peerdns flag is cleared only while static DNS is active;
#   * an invalid address never reaches network.* and is reported honestly;
#   * a missing IPv6 interface does not stop IPv4;
#   * repeated applications do not commit/reload again.
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
DNS="${ROOT}/root/usr/share/mt5700m/dns.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT INT TERM

mkdir -p "${TMP}/bin" "${TMP}/state"
export FAKE_UCI_STATE="${TMP}/uci.json"
export PATH="${TMP}/bin:${PATH}"

cp "${ROOT}/tests/helpers/uci" "${TMP}/bin/uci"
chmod 0755 "${TMP}/bin/uci"
printf '%s\n' '#!/bin/sh' 'exit 0' > "${TMP}/bin/logger"
chmod 0755 "${TMP}/bin/logger"
printf '%s\n' '#!/bin/sh' \
	'echo "$*" >> "${FAKE_UBUS_LOG:-/dev/null}"' 'exit 0' > "${TMP}/bin/ubus"
chmod 0755 "${TMP}/bin/ubus"
printf '%s\n' '#!/bin/sh' 'exit 0' > "${TMP}/bin/flock"
chmod 0755 "${TMP}/bin/flock"
export FAKE_UBUS_LOG="${TMP}/ubus.log"
: > "${FAKE_UBUS_LOG}"

seed() {
	cat > "${FAKE_UCI_STATE}" <<'EOF'
{
 "mt5700m.dns": "dns",
 "mt5700m.dns.ipv4_enabled": "1",
 "mt5700m.dns.ipv4_primary": "119.29.29.29",
 "mt5700m.dns.ipv4_secondary": "223.5.5.5",
 "mt5700m.dns.ipv6_enabled": "1",
 "mt5700m.dns.ipv6_primary": "2402:4e00::",
 "mt5700m.dns.ipv6_secondary": "2400:3200::1",
 "network.MT5700M": "interface",
 "network.MT5700M.proto": "dhcp",
 "network.MT5700Mv6": "interface",
 "network.MT5700Mv6.proto": "dhcpv6"
}
EOF
}

apply() {
	MT5700M_DNS_STATE_DIR="${TMP}/state" sh "${DNS}" apply
}

get() {
	FAKE_UCI_STATE="${FAKE_UCI_STATE}" "${TMP}/bin/uci" -q get "$1"
}

expect_eq() {
	[ "$1" = "$2" ] || {
		echo "FAIL: $3: expected '$2', got '$1'" >&2
		exit 1
	}
}

state_has() {
	grep -qx "$1" "${TMP}/state/dns.state" || {
		echo "FAIL: state missing '$1'" >&2
		cat "${TMP}/state/dns.state" >&2
		exit 1
	}
}

seed
apply
expect_eq "$(get network.MT5700M.dns)" '119.29.29.29 223.5.5.5' 'IPv4 resolvers'
expect_eq "$(get network.MT5700M.peerdns)" '0' 'IPv4 peerdns'
expect_eq "$(get network.MT5700Mv6.dns)" '2402:4e00:: 2400:3200::1' 'IPv6 resolvers'
expect_eq "$(get network.MT5700Mv6.peerdns)" '0' 'IPv6 peerdns'
state_has 'result=ok'
state_has 'v4_enabled=1'
state_has 'v4_servers=119.29.29.29 223.5.5.5'
state_has 'v6_enabled=1'
state_has 'v6_skipped=0'
expect_eq "$(grep -c 'network reload' "${FAKE_UBUS_LOG}")" '1' 'one network reload'

# Idempotent: no new commit/reload.
apply
expect_eq "$(grep -c 'network reload' "${FAKE_UBUS_LOG}")" '1' 'no reload when unchanged'

# Invalid IPv4 primary: default is written, the bad value is never injected.
seed
python3 - "${FAKE_UCI_STATE}" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["mt5700m.dns.ipv4_primary"] = "999.1.1.1"
json.dump(d, open(p, "w"))
PY
apply
expect_eq "$(get network.MT5700M.dns)" '119.29.29.29 223.5.5.5' 'fallback resolvers'
expect_eq "$(get mt5700m.dns.ipv4_primary)" '999.1.1.1' 'saved config untouched'
state_has 'result=partial'
grep -q 'IPv4 主 DNS 无效' "${TMP}/state/dns.state"

# Unspecified/multicast IPv4 literals parse as IPv4 but are not resolvers.
seed
python3 - "${FAKE_UCI_STATE}" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["mt5700m.dns.ipv4_primary"] = "0.0.0.0"
json.dump(d, open(p, "w"))
PY
apply
expect_eq "$(get network.MT5700M.dns)" '119.29.29.29 223.5.5.5' 'unspecified IPv4 rejected'
state_has 'result=partial'
grep -q 'IPv4 主 DNS 无效' "${TMP}/state/dns.state"

# Invalid IPv6 primary (too few groups, triple colon): the default is written
# and the malformed literal never reaches network.MT5700Mv6.
seed
python3 - "${FAKE_UCI_STATE}" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["mt5700m.dns.ipv6_primary"] = "2402:4e00:::1"
json.dump(d, open(p, "w"))
PY
apply
expect_eq "$(get network.MT5700Mv6.dns)" '2402:4e00:: 2400:3200::1' 'IPv6 fallback resolvers'
expect_eq "$(get mt5700m.dns.ipv6_primary)" '2402:4e00:::1' 'saved IPv6 config untouched'
state_has 'result=partial'
grep -q 'IPv6 主 DNS 无效' "${TMP}/state/dns.state"

# Missing IPv6 interface: IPv4 still applies; the skip is recorded.
seed
python3 - "${FAKE_UCI_STATE}" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
for k in [k for k in list(d) if k.startswith("network.MT5700Mv6")]:
    del d[k]
json.dump(d, open(p, "w"))
PY
apply
expect_eq "$(get network.MT5700M.dns)" '119.29.29.29 223.5.5.5' 'IPv4 applied without IPv6'
state_has 'result=ok'
state_has 'v6_skipped=1'

# IPv4 disabled: its resolvers and peerdns are removed, IPv6 still applied.
seed
python3 - "${FAKE_UCI_STATE}" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["mt5700m.dns.ipv4_enabled"] = "0"
d["network.MT5700M.dns"] = ["9.9.9.9"]
d["network.MT5700M.peerdns"] = "0"
json.dump(d, open(p, "w"))
PY
apply
if get network.MT5700M.dns >/dev/null 2>&1; then
	echo 'FAIL: IPv4 resolvers were not removed' >&2
	exit 1
fi
if get network.MT5700M.peerdns >/dev/null 2>&1; then
	echo 'FAIL: IPv4 peerdns was not removed' >&2
	exit 1
fi
expect_eq "$(get network.MT5700Mv6.dns)" '2402:4e00:: 2400:3200::1' 'IPv6 still applied'
state_has 'v4_enabled=0'
state_has 'v6_enabled=1'

# Missing section: defaults are created and saved.
rm -f "${FAKE_UCI_STATE}"
cat > "${FAKE_UCI_STATE}" <<'EOF'
{
 "network.MT5700M": "interface",
 "network.MT5700Mv6": "interface"
}
EOF
apply
expect_eq "$(get mt5700m.dns.ipv4_primary)" '119.29.29.29' 'defaults saved'
expect_eq "$(get network.MT5700M.dns)" '119.29.29.29 223.5.5.5' 'defaults applied'
state_has 'result=ok'

# A custom valid config survives repeated applies untouched.
seed
python3 - "${FAKE_UCI_STATE}" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["mt5700m.dns.ipv4_primary"] = "1.1.1.1"
d["mt5700m.dns.ipv4_secondary"] = "8.8.8.8"
json.dump(d, open(p, "w"))
PY
apply
apply
expect_eq "$(get mt5700m.dns.ipv4_primary)" '1.1.1.1' 'custom primary preserved'
expect_eq "$(get mt5700m.dns.ipv4_secondary)" '8.8.8.8' 'custom secondary preserved'
expect_eq "$(get network.MT5700M.dns)" '1.1.1.1 8.8.8.8' 'custom resolvers applied'

echo 'DNS apply tests passed'
