include $(TOPDIR)/rules.mk

# The WebUI (www/5700, from mt5700webui-openwrt-server 4.0, folded in at
# build time by scripts/build-release.sh) is a pre-built React/Webpack SPA
# whose bundle already ships minified.  LuCI's build-time `jsmin` (Crockford
# JSMin) cannot parse regex literals that follow `=>`/`)`/`]` and rewrites
# them as division, which corrupts the bundle and white-screens the SPA at
# /5700/.  Disable JS minification for this package so the already-minified
# bundle ships untouched.  The AT backend is a prebuilt static Rust binary
# (root/usr/bin/at-webserver, folded in at build time) — no Python runtime,
# no websockets/pyserial packages.
LUCI_MINIFY_JS:=0

# Same story for CSS: LuCI's build-time `csstidy --template=highest` merges
# selectors and drops "duplicate" rules.  Semi Design's theme relies on
# order-dependent repeated rules (e.g. `.semi-button-light` declared again
# after `.semi-button-primary` to restore the light background); csstidy
# collapses them, the solid primary background wins and every light/borderless
# button renders as a solid primary block with same-color (invisible) text.
# Ship the pre-minified vendor CSS untouched.
LUCI_MINIFY_CSS:=0

PKG_NAME:=luci-app-mt5700m
PKG_VERSION:=2.5.0
PKG_RELEASE:=1
PKG_LICENSE:=Apache-2.0

LUCI_TITLE:=MT5700M module management
# Refactored LuCI frontend (v2.5): ES2018+ client-side JavaScript built on the
# LuCI2 framework with a shared design system (resources/mt5700m/style.css),
# a component library (components.js), a unified data channel (api.js) and a
# pure AT parser layer (parser.js).  It uses `:root[data-darkmode]` theme
# variables, the L.resource() stylesheet loader and rpc.declare/fs.exec from
# luci-base >= 25.12.  OpenWrt < 25.12 (24.10 and earlier) is intentionally
# NOT supported; 25.12 is the current stable series and 24.10 reached EOL in
# September 2026.  NOTE: OpenWrt DEPENDS has no version-constraint syntax
# (unlike EXTRA_DEPENDS, which supports the one-word `(>=X)` form for ucode
# format deps) — a `pkg (>= X)` token is whitespace-split into separate
# pseudo-packages, breaks `make defconfig` with "dependency does not exist"
# warnings and a Kconfig syntax error.  The floor is therefore not enforced
# by the package Makefile; the feed-provided luci-base is expected to be >=
# 25.12.
LUCI_DEPENDS:=+luci-base +ubus-at-daemon +sms-tool_q
LUCI_EXTRA_DEPENDS:=kmod-usb2 (>=0), kmod-usb3 (>=0), kmod-usb-serial (>=0), kmod-usb-serial-option (>=0), kmod-usb-net (>=0), kmod-usb-net-cdc-ether (>=0), kmod-usb-net-cdc-ncm (>=0)
LUCI_PKGARCH:=all

define Package/luci-app-mt5700m/conffiles
/etc/mt5700m/traffic-history
endef

include $(TOPDIR)/feeds/luci/luci.mk

# call BuildPackage - OpenWrt buildroot signature
