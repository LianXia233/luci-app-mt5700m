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
# v3.2.0 — 架构 v2 收官（PR #7/#8/#9）：全页面读写路由化、双前端死代码清零、
# CI 前端证明链（cargo test 292）。发布构建默认关闭 LuCI 前端 JS 压缩
# （build-release.sh 不再调用 minify-luci-frontend.sh，安装包内 JS 保持源码
# 可读，需要时 MINIFY_LUCI_FRONTEND=1 恢复）。无 UI 行为变更。
#
# v3.1.2 — 版本号提升并触发重新编译（3.1.0 → 3.1.2，跳过已被 manual-v3.1.1
# 手动构建占用的 3.1.1），包含 v3.1.0 之后合入的模组状态视图共享异步状态重构。
#
# v3.1.0 — 异步化架构：后端读命令缓存闸门。渲染/轮询路径上的 AT 读命令
# 全部改成只读 raw 缓存（命中秒回、未命中提交后台采集并立刻返回占位），
# 前端刷新率与 AT 通道下发量彻底解耦。实测边际成本从 ~200ms/次 降到
# ~9ms/次（22x），并发 4 worker 混合三条路径平均 24ms/次。
# 同时修复 Mobile IP 卡片在 AT^NDISSTATQRY? 无应答时恒显 Disconnected。
#
# 沿用 v3.0.0 的 termios 直发方案（破坏性变更，保留在此备查）：AT 串口不再
# 依赖 BusyBox `stty` applet（OpenWrt 镜像普遍缺失），改由 Rust 直接发 termios
# ioctl；串口打开一律 O_NONBLOCK + VMIN=1/VTIME=0。旧版二进制在不修正
# termios 的情况下会把 TTY 留在 cooked 模式，AT 读取全部超时。
# 写命令、拨号、短信路径不变。
PKG_VERSION:=3.2.0
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
# Refactored backend (v2.6): the Rust `at-webserver` daemon owns the AT
# serial port EXCLUSIVELY (TIOCEXCL) and serves both frontends through a local
# control socket — `ubus-at-daemon` is no longer a dependency. SMS is built
# and sent in-process (pure-Rust PDU encoder), so `sms-tool_q` is also gone.
# Only luci-base is required.
LUCI_DEPENDS:=+luci-base
LUCI_EXTRA_DEPENDS:=kmod-usb2 (>=0), kmod-usb3 (>=0), kmod-usb-serial (>=0), kmod-usb-serial-option (>=0), kmod-usb-net (>=0), kmod-usb-net-cdc-ether (>=0), kmod-usb-net-cdc-ncm (>=0)
LUCI_PKGARCH:=all

define Package/luci-app-mt5700m/conffiles
/etc/mt5700m/traffic-history
endef

include $(TOPDIR)/feeds/luci/luci.mk

# call BuildPackage - OpenWrt buildroot signature
