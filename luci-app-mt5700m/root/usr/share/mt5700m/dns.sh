#!/bin/sh
# Single DNS apply path for the MT5700M plugin.
#
# Every plugin entry point that can bring the cellular interfaces up calls this
# script (the manager's ensure_network, the init script and the LuCI apply
# action). It writes the configured resolvers only to the two interfaces this
# plugin owns:
#
#   MT5700M    IPv4 / DHCP
#   MT5700Mv6  IPv6 / DHCPv6
#
# It never edits the global resolver, LAN/WAN, dnsmasq, firewall or any other
# interface. When netifd reads these options, it hands the resolvers to the
# resolver for those interfaces only.
#
# Options are read from UCI on every invocation, so a saved change is used by
# the next startup/up even if the daemon never ran. Defaults are applied when
# the section is missing, but an existing valid value is never overwritten.

set -u

SECTION='mt5700m.dns'
IFACE4='MT5700M'
IFACE6='MT5700Mv6'
STATE_DIR="${MT5700M_DNS_STATE_DIR:-/var/run/mt5700m}"
STATE_FILE="${STATE_DIR}/dns.state"
LOCK_FILE="${STATE_DIR}/dns.lock"

DEFAULT_V4_PRIMARY='119.29.29.29'
DEFAULT_V4_SECONDARY='223.5.5.5'
DEFAULT_V6_PRIMARY='2402:4e00::'
DEFAULT_V6_SECONDARY='2400:3200::1'

TAG='mt5700m-dns'

log() {
	logger -t "${TAG}" -- "$*" 2>/dev/null || true
}

valid_ipv4() {
	local ip="$1" o oldIFS
	case "${ip}" in
		''|*[!0-9.]*) return 1 ;;
	esac
	oldIFS="${IFS}"
	IFS='.'
	# shellcheck disable=SC2086
	set -- ${ip}
	IFS="${oldIFS}"
	[ "$#" -eq 4 ] || return 1
	for o in "$@"; do
		case "${o}" in
			''|*[!0-9]*) return 1 ;;
			0?*) return 1 ;;
		esac
		[ "${#o}" -le 3 ] || return 1
		[ "${o}" -le 255 ] || return 1
	done
	# Match the backend: unspecified, multicast and broadcast are not
	# resolvers even though they parse as IPv4 literals.
	[ "$1" -ne 0 ] || return 1
	[ "$1" -lt 224 ] || return 1
	return 0
}

valid_ipv6() {
	awk -v ip="$1" '
	function v4(s,   a, n, i) {
		n = split(s, a, ".")
		if (n != 4) return 0
		for (i = 1; i <= n; i++) {
			if (a[i] !~ /^[0-9]+$/ || length(a[i]) > 3 || a[i] + 0 > 255) return 0
		}
		return 1
	}
	BEGIN {
		if (ip == "" || ip ~ /%/) exit 1
		if (ip ~ /[^0-9A-Fa-f:.]/) exit 1
		# Unspecified and multicast (ff00::/8) are rejected like the backend.
		if (ip == "::") exit 1
		first = ip
		sub(/:.*/, "", first)
		if (tolower(first) ~ /^ff[0-9a-f][0-9a-f]$/) exit 1
		if (gsub(/::/, "::", ip) > 1) exit 1
		dbl = index(ip, "::")
		if (dbl > 0) {
			head = substr(ip, 1, dbl - 1)
			tail = substr(ip, dbl + 2)
		} else {
			head = ip
			tail = ""
		}
		if (head ~ /^:/ || head ~ /:$/) exit 1
		if (tail ~ /^:/ || tail ~ /:$/) exit 1
		groups = 0
		if (head != "") {
			n = split(head, a, ":")
			for (i = 1; i <= n; i++) {
				if (a[i] == "" || length(a[i]) > 4 || a[i] !~ /^[0-9A-Fa-f]+$/) exit 1
				groups++
			}
		}
		if (tail != "") {
			n = split(tail, a, ":")
			for (i = 1; i <= n; i++) {
				if (i == n && a[i] ~ /\./) {
					if (!v4(a[i])) exit 1
					groups += 2
				} else {
					if (a[i] == "" || length(a[i]) > 4 || a[i] !~ /^[0-9A-Fa-f]+$/) exit 1
					groups++
				}
			}
		}
		# "::" must compress at least one zero group; without it exactly
		# eight 16-bit groups are required.
		if (dbl > 0) {
			if (groups > 7) exit 1
		} else if (groups != 8) exit 1
		exit 0
	}'
}

uci_ok() {
	uci -q get "$1" >/dev/null 2>&1
}

# --- lock -----------------------------------------------------------------
# A hotplug/manual apply can race the manager's periodic tick. Serialise them
# so two `network reload`s never overlap. Busybox flock is present on target;
# when it is missing we still proceed (the script is idempotent).
mkdir -p "${STATE_DIR}" 2>/dev/null || true
if command -v flock >/dev/null 2>&1; then
	exec 9>"${LOCK_FILE}" 2>/dev/null || true
	flock -x 9 2>/dev/null || true
fi

write_state() {
	local result="$1" error="$2" v4_en="$3" v4_servers="$4" v6_en="$5" v6_servers="$6" v6_skipped="$7"
	local tmp="${STATE_FILE}.$$"
	mkdir -p "${STATE_DIR}" 2>/dev/null || true
	{
		printf 'result=%s\n' "${result}"
		printf 'applied_at=%s\n' "$(date +%s 2>/dev/null || echo 0)"
		printf 'v4_enabled=%s\n' "${v4_en}"
		printf 'v4_servers=%s\n' "${v4_servers}"
		printf 'v6_enabled=%s\n' "${v6_en}"
		printf 'v6_servers=%s\n' "${v6_servers}"
		printf 'v6_skipped=%s\n' "${v6_skipped}"
		printf 'error=%s\n' "$(printf '%s' "${error}" | tr '\n' ' ')"
	} > "${tmp}" 2>/dev/null || {
		rm -f "${tmp}" 2>/dev/null
		return 1
	}
	mv "${tmp}" "${STATE_FILE}" 2>/dev/null || {
		rm -f "${tmp}" 2>/dev/null
		return 1
	}
}

# Create the section with defaults only when it does not exist at all. An
# existing section is authoritative, even if a value is unusual.
ensure_section() {
	uci_ok "${SECTION}" && return 0
	uci -q set "${SECTION}=dns" || return 1
	uci -q set "${SECTION}.ipv4_enabled=1"
	uci -q set "${SECTION}.ipv4_primary=${DEFAULT_V4_PRIMARY}"
	uci -q set "${SECTION}.ipv4_secondary=${DEFAULT_V4_SECONDARY}"
	uci -q set "${SECTION}.ipv6_enabled=1"
	uci -q set "${SECTION}.ipv6_primary=${DEFAULT_V6_PRIMARY}"
	uci -q set "${SECTION}.ipv6_secondary=${DEFAULT_V6_SECONDARY}"
	uci -q commit mt5700m || return 1
	return 0
}

cfg_bool() {
	# $1 option, $2 default
	local value
	value="$(uci -q get "${SECTION}.$1" 2>/dev/null || true)"
	case "${value}" in
		'') echo "$2" ;;
		0|false|off|no|disabled) echo 0 ;;
		*) echo 1 ;;
	esac
}

cfg_text() {
	uci -q get "${SECTION}.$1" 2>/dev/null || printf '%s' "$2"
}

normalise() {
	# shellcheck disable=SC2086
	set -- $1
	printf '%s' "$*"
}

changed=0

# apply_family <iface> <enabled> <primary> <secondary> <family-label>
# returns 0 applied, 1 failed, 2 skipped (IPv6 interface absent)
apply_family() {
	local iface="$1" enabled="$2" primary="$3" secondary="$4" label="$5"
	local section current peer wanted="" value
	section="$(uci -q get "network.${iface}" 2>/dev/null || true)"
	if [ -z "${section}" ]; then
		# A disabled family has nothing to remove on an absent interface.
		[ "${enabled}" = '0' ] && return 0
		if [ "${iface}" = "${IFACE6}" ]; then
			return 2
		fi
		log "${label}: network.${iface} is not configured"
		return 1
	fi

	if [ "${enabled}" = '1' ]; then
		for value in "${primary}" "${secondary}"; do
			[ -n "${value}" ] || continue
			case " ${wanted} " in
				*" ${value} "*) ;;
				*) wanted="${wanted:+${wanted} }${value}" ;;
			esac
		done
		if [ -z "${wanted}" ]; then
			log "${label}: enabled but no server configured"
			return 1
		fi
	fi

	current="$(uci -q get "network.${iface}.dns" 2>/dev/null || true)"
	peer="$(uci -q get "network.${iface}.peerdns" 2>/dev/null || true)"
	if [ "${enabled}" = '1' ]; then
		if [ "$(normalise "${current}")" != "${wanted}" ] || [ "${peer}" != '0' ]; then
			uci -q delete "network.${iface}.dns" 2>/dev/null || true
			for value in ${wanted}; do
				uci -q add_list "network.${iface}.dns=${value}" || return 1
			done
			uci -q set "network.${iface}.peerdns=0" || return 1
			changed=1
		fi
	else
		if [ -n "${current}" ] || [ -n "${peer}" ]; then
			uci -q delete "network.${iface}.dns" 2>/dev/null || true
			uci -q delete "network.${iface}.peerdns" 2>/dev/null || true
			changed=1
		fi
	fi
	return 0
}

do_apply() {
	local v4_en v4p v4s v6_en v6p v6s v4_want v6_want v4_result v6_result v6_skipped errors result
	local state_v4_en state_v4_servers state_v6_en state_v6_servers

	if ! command -v uci >/dev/null 2>&1; then
		write_state error 'uci is not available' 0 '' 0 '' 0
		echo 'uci is not available' >&2
		return 1
	fi

	ensure_section || {
		write_state error 'cannot create mt5700m.dns section' 0 '' 0 '' 0
		echo 'cannot create DNS section' >&2
		return 1
	}

	v4_en="$(cfg_bool ipv4_enabled 1)"
	v4p="$(cfg_text ipv4_primary "${DEFAULT_V4_PRIMARY}")"
	v4s="$(cfg_text ipv4_secondary "${DEFAULT_V4_SECONDARY}")"
	v6_en="$(cfg_bool ipv6_enabled 1)"
	v6p="$(cfg_text ipv6_primary "${DEFAULT_V6_PRIMARY}")"
	v6s="$(cfg_text ipv6_secondary "${DEFAULT_V6_SECONDARY}")"

	errors=''
	if [ "${v4_en}" = '1' ]; then
		if [ -z "${v4p}" ]; then
			v4p="${DEFAULT_V4_PRIMARY}"
			errors="${errors}; IPv4 主 DNS 为空，已使用默认值"
		elif ! valid_ipv4 "${v4p}"; then
			v4p="${DEFAULT_V4_PRIMARY}"
			errors="${errors}; IPv4 主 DNS 无效，已使用默认值"
		fi
		if [ -n "${v4s}" ] && ! valid_ipv4 "${v4s}"; then
			v4s=''
			errors="${errors}; IPv4 备用 DNS 无效，已忽略"
		fi
		v4_want="$(normalise "${v4p} ${v4s}")"
	else
		v4_want=''
	fi

	if [ "${v6_en}" = '1' ]; then
		if [ -z "${v6p}" ]; then
			v6p="${DEFAULT_V6_PRIMARY}"
			errors="${errors}; IPv6 主 DNS 为空，已使用默认值"
		elif ! valid_ipv6 "${v6p}"; then
			v6p="${DEFAULT_V6_PRIMARY}"
			errors="${errors}; IPv6 主 DNS 无效，已使用默认值"
		fi
		if [ -n "${v6s}" ] && ! valid_ipv6 "${v6s}"; then
			v6s=''
			errors="${errors}; IPv6 备用 DNS 无效，已忽略"
		fi
		v6_want="$(normalise "${v6p} ${v6s}")"
	else
		v6_want=''
	fi

	apply_family "${IFACE4}" "${v4_en}" "${v4p}" "${v4s}" 'IPv4'
	v4_result=$?
	case "${v4_result}" in
		0) : ;;
		2) errors="${errors}; IPv4 接口不存在" ;;
		*) errors="${errors}; IPv4 应用失败" ;;
	esac

	apply_family "${IFACE6}" "${v6_en}" "${v6p}" "${v6s}" 'IPv6'
	v6_result=$?
	v6_skipped=0
	case "${v6_result}" in
		0) : ;;
		2) v6_skipped=1 ;;
		*) errors="${errors}; IPv6 应用失败" ;;
	esac

	errors="${errors#; }"

	if [ "${changed}" = '1' ]; then
		uci -q commit network || {
			write_state error 'cannot commit network config' 0 '' 0 '' 0
			echo 'cannot commit network config' >&2
			return 1
		}
		if command -v ubus >/dev/null 2>&1; then
			ubus call network reload >/dev/null 2>&1 || /etc/init.d/network reload >/dev/null 2>&1 || true
		else
			/etc/init.d/network reload >/dev/null 2>&1 || true
		fi
	fi

	if [ "${v4_result}" != '0' ]; then
		result='error'
	elif [ "${v6_result}" != '0' ] && [ "${v6_result}" != '2' ]; then
		result='partial'
	elif [ -n "${errors}" ]; then
		result='partial'
	else
		result='ok'
	fi

	# Report what was actually written, not what was requested: a rejected
	# value was replaced by a default above. A skipped IPv6 interface keeps
	# the intended values in the state (v6_skipped=1 tells the UI they are
	# not live yet) so a transient absence is not reported as a pending
	# configuration change.
	state_v4_en=0
	state_v4_servers=''
	if [ "${v4_en}" = '1' ] && [ "${v4_result}" = '0' ]; then
		state_v4_en=1
		state_v4_servers="${v4_want}"
	fi
	state_v6_en=0
	state_v6_servers=''
	if [ "${v6_en}" = '1' ] && [ "${v6_result}" != '1' ]; then
		state_v6_en=1
		state_v6_servers="${v6_want}"
	fi
	write_state "${result}" "${errors}" "${state_v4_en}" "${state_v4_servers}" \
		"${state_v6_en}" "${state_v6_servers}" "${v6_skipped}"
	log "apply result=${result} v4=${v4_want:-disabled} v6=${v6_want:-disabled} skipped=${v6_skipped} error=${errors:-none} changed=${changed}"
	[ "${result}" != 'error' ]
}

do_status() {
	if [ -r "${STATE_FILE}" ]; then
		cat "${STATE_FILE}"
	else
		echo 'result=never'
	fi
}

case "${1:-apply}" in
	apply) do_apply ;;
	status) do_status ;;
	*)
		echo "Usage: $0 {apply|status}" >&2
		exit 64
		;;
esac
exit $?
