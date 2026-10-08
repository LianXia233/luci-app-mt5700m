#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work_dir="${RUNNER_TEMP:-/tmp}/mt5700m-sdk"
output_dir="${repo_dir}/dist-release"
base_url="https://downloads.openwrt.org/snapshots/targets/mediatek/filogic"

mkdir -p "${work_dir}" "${output_dir}"
find "${output_dir}" -mindepth 1 -maxdepth 1 -delete
cd "${work_dir}"

# SDK downloads from downloads.openwrt.org are flaky: the connection is
# dropped mid-stream partway through the ~263 MB tarball, surfacing as
#   curl: (92) HTTP/2 stream 1 was not closed cleanly: PROTOCOL_ERROR
#   curl: (18) transfer closed with N bytes remaining to read
# A plain `--retry` cannot ride this out: curl restarts from byte 0 on every
# attempt, so a link that reliably dies at ~92% never completes no matter how
# many times it is retried (observed: six attempts all aborted in the last
# ~25 MB).  The server advertises `Accept-Ranges: bytes`, so instead we loop
# with `-C -` (resume).  Each round keeps whatever bytes already landed and
# asks for the rest; the tarball always finishes as long as the link makes
# forward progress.  --http1.1 avoids the HTTP/2 multiplexing bug entirely.
download() {
	local url="$1" out="$2" want="${3:-0}"
	local round size after rc stall=0
	# With no explicit size, ask the server what it should be.  Without this
	# the completion check can never fire (`want=0` is never `>=`), and a file
	# that DID download completely would keep looping until the round cap —
	# the exact failure seen on sha256sums (26,336,446 bytes fetched, then 54
	# wasted rounds because nothing ever compared against 26,336,446).
	if [ "${want}" -eq 0 ]; then
		want="$(curl -sSI --http1.1 --max-time 30 "${url}" \
			| awk 'tolower($1)=="content-length:" { gsub(/\r/,"",$2); print $2; exit }')"
		want="${want:-0}"
		echo "   expected size: ${want} bytes (from Content-Length)"
	fi
	# Skip entirely if a complete copy is already on disk (size known).
	if [ "${want}" -gt 0 ] && [ -f "${out}" ] \
	   && [ "$(stat -c%s "${out}" 2>/dev/null || echo 0)" -ge "${want}" ]; then
		echo "   ${out} already complete ($(stat -c%s "${out}") bytes)"
		return 0
	fi
	for round in $(seq 1 60); do
		size=0
		[ -f "${out}" ] && size="$(stat -c%s "${out}" 2>/dev/null || echo 0)"
		if [ "${want}" -gt 0 ] && [ "${size}" -ge "${want}" ]; then
			echo "   done after ${round} round(s): ${size}/${want} bytes"
			return 0
		fi
		rc=0
		curl -sS --http1.1 --fail --location -C - --max-time 300 \
			--connect-timeout 30 -o "${out}" "${url}" || rc=$?
		after=0
		[ -f "${out}" ] && after="$(stat -c%s "${out}" 2>/dev/null || echo 0)"
		echo "   round ${round}: rc=${rc}  ${size} -> ${after}${want:+/${want}} bytes"
		# curl exits 33 when the server refuses a range request because the
		# file is already complete — treat that as success, not a stall.
		if [ "${rc}" -eq 33 ] && [ "${want}" -gt 0 ] && [ "${after}" -ge "${want}" ]; then
			return 0
		fi
		# Clean transfer and the file is non-empty: the server closed the
		# body on its own, which for these snapshots means "done" even if the
		# advertised size was unavailable.  Never loop on a no-op.
		if [ "${rc}" -eq 0 ] && [ "${after}" -gt 0 ] && [ "${after}" -eq "${size}" ]; then
			if [ "${want}" -eq 0 ] || [ "${after}" -ge "${want}" ]; then
				echo "   complete at ${after} bytes (server closed cleanly)"
				return 0
			fi
		fi
		# No forward progress three rounds in a row ⇒ the link is dead, stop
		# instead of burning the whole job timeout on hopeless retries.
		if [ "${after}" -le "${size}" ] && [ "${rc}" -ne 0 ]; then
			stall=$(( stall + 1 ))
			if [ "${stall}" -ge 3 ]; then
				echo "ERROR: download stalled with no progress: ${url}" >&2
				return 1
			fi
		else
			stall=0
		fi
		sleep 2
	done
	echo "ERROR: download did not finish in 60 rounds: ${url}" >&2
	return 1
}

# Grab the checksum manifest first; `download` reads its Content-Length so it
# can tell "finished" from "truncated".  A resumed file that ends up the right
# size but corrupt (segments stitched after several drops) is caught by the
# sanity grep below, which drops the file and refetches from scratch.
download "${base_url}/sha256sums" sha256sums
if ! grep -q 'openwrt-sdk-.*Linux-x86_64\.tar\.zst' sha256sums 2>/dev/null; then
	echo "WARNING: sha256sums looks truncated/corrupt, refetching from scratch" >&2
	rm -f sha256sums
	download "${base_url}/sha256sums" sha256sums
fi
test -s sha256sums
archive="$(awk '/openwrt-sdk-.*Linux-x86_64\.tar\.zst$/ { print $2; exit }' sha256sums | sed 's/^\*//')"
test -n "${archive}"
echo "SDK tarball: ${archive}"
download "${base_url}/${archive}" "${archive}"
grep "[ *]${archive}$" sha256sums | sha256sum -c -
tar --zstd -xf "${archive}"
sdk_dir="$(find "${work_dir}" -maxdepth 1 -type d -name 'openwrt-sdk-*' | head -n 1)"
test -n "${sdk_dir}"

# NOTE (v2.6): the `qmodem` feed is no longer pulled in — it only supplied
# ubus-at-daemon and sms-tool_q, both eliminated by the Rust backend
# (exclusive serial + local control socket + in-process SMS encoder).
cd "${sdk_dir}"
./scripts/feeds update -a
./scripts/feeds install luci-base

perl -0pi -e 's/(config ALL\n\s+bool "Select all userspace packages by default"\n\s+default )y/${1}n/' Config.in
perl -0pi -e 's/(config TARGET_MULTI_PROFILE\n\s+bool\n\s+default )y/${1}n/; s/(config TARGET_ALL_PROFILES\n\s+bool\n\s+default )y/${1}n/; s/(config TARGET_DEVICE_mediatek_filogic_DEVICE_[^\n]+\n\s+bool\n\s+default )y/${1}n/g' Config-build.in
sed -i 's/^[[:space:]]*default m$/\tdefault n/' Config-build.in

mkdir -p package/h5000m-custom
cp -a "${repo_dir}/luci-app-mt5700m" package/h5000m-custom/

# ---------------------------------------------------------------------------
# Build the Rust AT backend (v4.0). One std-only static binary serves BOTH
# frontends: "at-webserver" (WebSocket daemon for the WebUI) and
# "mt5700m-at" (LuCI shell contract via argv[0] dispatch; installed as a
# symlink by 93-mt5700m-webui). The aarch64-unknown-linux-musl target links
# with the bundled rust-lld, so no cross toolchain is needed on the runner.
# NOTE: linker config is MANDATORY. Without it rustc drives the HOST cc, and
# the aarch64-only workaround flag `-Wl,--fix-cortex-a53-843419` (injected by
# rustc for this target) is rejected by the x86_64 GNU ld:
#   /usr/bin/ld: unrecognized option '--fix-cortex-a53-843419'
# rust-lld understands it and links self-contained (bundled musl crt + libc).
# ---------------------------------------------------------------------------
rust_dir="${repo_dir}/mt5700webui-openwrt-server/at-webserver"
rust_target="aarch64-unknown-linux-musl"
rust_bin=""
if command -v cargo >/dev/null 2>&1; then
	rustup target add "${rust_target}" >/dev/null 2>&1 || true
	(cd "${rust_dir}" && \
	 RUSTFLAGS="-C link-self-contained=yes -C linker=rust-lld" \
	 cargo build --release --locked --target "${rust_target}")
	rust_bin="${rust_dir}/target/${rust_target}/release/at-webserver"
fi
if [ ! -f "${rust_bin}" ]; then
	echo "ERROR: Rust backend binary not built (cargo missing or compile error)." >&2
	exit 1
fi
echo "INFO: built Rust at-webserver backend (${rust_target})"

# Fold the standalone WebUI (mt5700webui 4.0: React/Semi frontend + Rust
# AT backend) into the package source, so one apk ships frontend + backend +
# LuCI manager.  The LuCI app itself no longer carries the old umi WebUI
# (htdocs/5700, at-server.py were removed from the repo).
pkg_src="package/h5000m-custom/luci-app-mt5700m"
mkdir -p "${pkg_src}/htdocs" "${pkg_src}/root/usr/bin" "${pkg_src}/root/etc/init.d"
cp -a "${repo_dir}/mt5700webui-openwrt-server/at-webserver/files/www/5700" "${pkg_src}/htdocs/5700"
# The WebUI discovers its AT WebSocket endpoint at runtime, in this order:
#   1) GET /cgi-bin/at-ws-info  (dynamic: returns the host the client is
#      actually talking to, so any LAN subnet works)
#   2) fallback: /5700/config.json, whose `at.host` is a build-time default of
#      192.168.1.1
# Only `www/5700` used to be folded in, so at-ws-info was missing from the
# package (404) and every installation silently fell back to the hard-coded
# 192.168.1.1 — on any other subnet (e.g. 192.168.10.1) the WebSocket never
# connects and /5700 renders no data at all. Ship the CGI scripts too.
mkdir -p "${pkg_src}/htdocs/cgi-bin"
cp -a "${repo_dir}/mt5700webui-openwrt-server/at-webserver/files/www/cgi-bin/." "${pkg_src}/htdocs/cgi-bin/."
chmod 0755 "${pkg_src}/htdocs/cgi-bin/"*
cp -f "${rust_bin}" "${pkg_src}/root/usr/bin/at-webserver"
chmod 0755 "${pkg_src}/root/usr/bin/at-webserver"
cp -f "${repo_dir}/mt5700webui-openwrt-server/at-webserver/files/etc/init.d/at-webserver" "${pkg_src}/root/etc/init.d/at-webserver"
echo "INFO: folded mt5700webui 4.0 frontend + Rust backend into package source"

# LuCI frontend minification is DISABLED by default since v3.2.0: release
# artifacts ship readable JS/CSS so on-device debugging, diffing and grepping
# stay possible.  The esbuild minifier itself is still proven on every CI run
# (static-checks + frontend-proofs both exercise
# scripts/minify-luci-frontend.sh), so re-enabling it is a one-liner:
#   MINIFY_LUCI_FRONTEND=1 ./scripts/build-release.sh
# The script runs AFTER the /5700 fold and only touches
# luci-static/resources/{mt5700m,view/mt5700m}, so the React bundle stays
# pristine either way.  esbuild + node must be on PATH (both are present on
# the CI runners; node --check validates every minified output).
if [ "${MINIFY_LUCI_FRONTEND:-0}" = "1" ]; then
	bash "${repo_dir}/scripts/minify-luci-frontend.sh" "${pkg_src}/htdocs"
	echo "INFO: minified LuCI frontend (mt5700m views/modules/style.css)"
else
	echo "INFO: LuCI frontend left unminified (set MINIFY_LUCI_FRONTEND=1 to enable)"
fi

cat > .config <<'EOF'
CONFIG_TARGET_mediatek=y
CONFIG_TARGET_mediatek_filogic=y
# CONFIG_ALL is not set
# CONFIG_ALL_KMODS is not set
# CONFIG_ALL_NONSHARED is not set
CONFIG_PACKAGE_luci-app-mt5700m=m
CONFIG_LUCI_LANG_zh_Hans=y
# CONFIG_PACKAGE_ubus-at-daemon is not set
# CONFIG_PACKAGE_sms-tool_q is not set
# CONFIG_PACKAGE_luci-app-qmodem is not set
# CONFIG_PACKAGE_luci-app-qmodem-next is not set
# CONFIG_PACKAGE_qmodem is not set
# CONFIG_PACKAGE_modem_scan is not set
# CONFIG_PACKAGE_tom_modem is not set
EOF
make defconfig
# Force a clean rebuild so the SDK re-copies the updated htdocs (network.js/status.js)
# instead of reusing a cached build_dir / staging copy from the previous version.
make package/h5000m-custom/luci-app-mt5700m/clean >/dev/null 2>&1 || true
rm -rf build_dir/target-*/luci-app-mt5700m \
       staging_dir/target-*/root-*/www/luci-static/resources/view/mt5700m \
       staging_dir/target-*/root-*/www/5700 \
       bin/packages/*/custom/luci-app-mt5700m*.apk 2>/dev/null || true
# CRLF prevention: .gitattributes mandates eol=lf for www/5700 text files.
# A pre-compile `sed -i 's/\r$//'` was empirically proven to corrupt large
# single-line JS bundles on CI runners, so it stays removed.  The post-compile
# `cp -a` of pristine repo files into staging_dir is the safety net for any
# SDK copy/tar artifacts, and `node --check` validates the result.

make package/h5000m-custom/luci-app-mt5700m/compile -j"$(nproc)" V=s

# Re-copy the PRISTINE www/5700 frontend (mt5700webui 4.0) from the repo
# source into the freshly staged www tree, AFTER `make compile` and BEFORE
# the node --check guard below.
#
# This is the step that fixes SDK truncation of huge minified JS bundles
# (34000+ char lines; the SDK copy/tar mangles CRLF/long-line files).
# Empirical history with the old umi bundle:
#   - v2.3.22: step PRESENT  -> build SUCCEEDED
#   - v2.3.26: step REMOVED  -> build FAILED (node --check caught truncation)
#   - v2.3.27: absent again  -> build FAILED
# The .apk is assembled FROM staging_dir, so overwriting staging_dir here
# DOES reach the package.  The new React bundle has the same exposure.
cp -a "${repo_dir}/mt5700webui-openwrt-server/at-webserver/files/www/5700/." staging_dir/target-*/root-*/www/5700/.
echo "INFO: re-copied pristine www/5700 (mt5700webui 4.0) into staging_dir after compile"

# Sanity check: the freshly staged www tree must contain the WebUI integration.
# If this fails, the SDK reused a cached htdocs copy and the package would be broken.
# We check for the homepage entry-button class (a JS string literal that survives
# any minification, unlike a // comment) and for the bundled WebUI SPA entry.
# Literal: the refactored LuCI frontend (v2.5, status.js) marks the /5700 entry
# button with class `mt-hero-btn`; older releases used `mt5700m-webui-cta`.
if ! grep -rq "mt-hero-btn" staging_dir/target-*/root-*/www/luci-static/resources/view/mt5700m/ 2>/dev/null; then
  echo "ERROR: built www tree is missing the WebUI entry button (SDK caching?)" >&2
  exit 1
fi
if ! ls staging_dir/target-*/root-*/www/5700/index.html >/dev/null 2>&1; then
  echo "ERROR: built www tree is missing the WebUI SPA at /www/5700/index.html" >&2
  exit 1
fi
# Guard rail: catch truncated/garbled JS bundles (e.g. broken regex) before packaging.
# IMPORTANT: validate EVERY .js under /www/5700/, not just the main umi bundle.
# A truncated per-route async chunk (e.g. p__CPE__Network__Info__index.*.async.js)
# parses with a SyntaxError and white-screens ONLY that route while the rest of the
# app loads fine — exactly the symptom reported for /network/info. The earlier guard
# only checked umi.ec9b4b52.js and let broken route chunks through.
while IFS= read -r js; do
  [ -f "$js" ] || continue
  if ! node --check "$js" 2>/dev/null; then
    echo "ERROR: $js has syntax errors (likely truncated by SDK build)" >&2
    exit 1
  fi
done < <(find staging_dir/target-*/root-*/www/5700 -name '*.js' -type f 2>/dev/null)

find bin -type f \( -name 'luci-app-mt5700m-*.apk' -o -name 'luci-app-mt5700m_*.ipk' -o -name 'luci-i18n-mt5700m-zh-cn-*.apk' -o -name 'luci-i18n-mt5700m-zh-cn_*.ipk' \) -exec cp -f {} "${output_dir}/" \;
# v2.6 removed the ubus-at-daemon / sms-tool_q third-party packages, so the
# release now ships exactly two packages (LuCI app + zh-cn i18n). Verify both
# reached the output instead of hard-coding a legacy artifact count (was -ge 4
# when two extra packages still existed), which would fail silently under
# `set -e` on any future artifact change.
# NOTE: use `find` (not `ls "${dir}"/glob-*.ipk`) because in apk-only builds the
# .ipk glob has no match and bash passes it through literally, making ls fail
# and falsely reporting the package as missing.
app_count="$(find "${output_dir}" -maxdepth 1 -type f \( -name 'luci-app-mt5700m-*.apk' -o -name 'luci-app-mt5700m_*.ipk' \) | wc -l)"
i18n_count="$(find "${output_dir}" -maxdepth 1 -type f \( -name 'luci-i18n-mt5700m-zh-cn-*.apk' -o -name 'luci-i18n-mt5700m-zh-cn_*.ipk' \) | wc -l)"
if [ "${app_count}" -eq 0 ]; then
  echo "ERROR: build output is missing the luci-app-mt5700m package (no .apk/.ipk)" >&2
  exit 1
fi
if [ "${i18n_count}" -eq 0 ]; then
  echo "ERROR: build output is missing the luci-i18n-mt5700m-zh-cn package (no .apk/.ipk)" >&2
  exit 1
fi
cp -f public-key.pem "${output_dir}/openwrt-sdk-build.pem" 2>/dev/null || true
(cd "${output_dir}" && find . -maxdepth 1 -type f \( -name '*.apk' -o -name '*.ipk' -o -name 'openwrt-sdk-build.pem' \) -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS)
