#!/usr/bin/env bash
# Minify the LuCI frontend of luci-app-mt5700m (NOT the folded /5700 SPA).
#
# Why this exists: the package sets LUCI_MINIFY_JS:=0 / LUCI_MINIFY_CSS:=0
# because luci.mk's Crockford jsmin and csstidy corrupt the prebuilt React
# bundle folded into htdocs/5700 (regex-after-'=>' rewrite, order-dependent
# Semi Design rules).  The side effect was that the LuCI views themselves
# (view/mt5700m + mt5700m shared modules + style.css, ~200 KB) shipped
# completely unminified.  This script minifies ONLY the mt5700m LuCI resource
# tree with esbuild, which parses modern JS correctly (top-level `return` of
# LuCI module files included) and preserves the leading quoted `require ...`
# statements that LuCI's module loader scans for.
#
# Usage:
#   scripts/minify-luci-frontend.sh [htdocs-dir]
#   htdocs-dir defaults to luci-app-mt5700m/htdocs of this repository.
#   The tree is minified IN PLACE; run it on a build copy, not on the git tree.
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
htdocs="${1:-${repo_dir}/luci-app-mt5700m/htdocs}"
res_dir="${htdocs}/luci-static/resources"

die() { echo "ERROR: $*" >&2; exit 1; }

[ -d "${res_dir}/mt5700m" ] && [ -d "${res_dir}/view/mt5700m" ] \
	|| die "not a luci-app-mt5700m htdocs tree: ${htdocs}"
command -v node >/dev/null 2>&1 || die "node is required for validation"
command -v npx >/dev/null 2>&1 || die "npx (npm) is required to run esbuild"

# Count the quoted `require ...` directives LuCI's loader depends on.
# `|| true`: pipefail must not abort on zero matches (CSS files).
count_requires() {
	grep -oE '["'\'']require [^"'\'']+["'\'']' "$1" 2>/dev/null | wc -l | tr -d ' ' || true
}

minify_one() {
	local src="$1" ext tmp before after req_before req_after
	ext="${src##*.}"
	tmp="${src%.${ext}}.min.${ext}"
	before=$(wc -c < "${src}" | tr -d ' ')
	req_before=$(count_requires "${src}")

	npx --yes esbuild "${src}" --minify --outfile="${tmp}" --log-level=error \
		|| die "esbuild failed on ${src}"
	if [ "${ext}" = "js" ]; then
		node --check "${tmp}" || die "node --check failed on ${tmp}"
	fi

	req_after=$(count_requires "${tmp}")
	[ "${req_after}" -eq "${req_before}" ] \
		|| die "require directives lost in ${src} (${req_before} -> ${req_after})"

	mv "${tmp}" "${src}"
	after=$(wc -c < "${src}" | tr -d ' ')
	echo "  $(basename "$(dirname "${src}")")/$(basename "${src}"): ${before} -> ${after} bytes"
}

echo "Minifying LuCI frontend in ${res_dir}"

total_before=0
total_after=0
for f in "${res_dir}"/mt5700m/*.js "${res_dir}"/mt5700m/*.css "${res_dir}"/view/mt5700m/*.js; do
	[ -f "$f" ] || continue
	b=$(wc -c < "$f" | tr -d ' ')
	minify_one "$f"
	a=$(wc -c < "$f" | tr -d ' ')
	total_before=$((total_before + b))
	total_after=$((total_after + a))
done

[ "${total_before}" -gt 0 ] || die "no files matched, nothing minified"
saved=$(( (total_before - total_after) * 100 / total_before ))
echo "Total: ${total_before} -> ${total_after} bytes (-${saved}%)"
