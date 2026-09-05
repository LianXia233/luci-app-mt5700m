include $(TOPDIR)/rules.mk

# The WebUI (www/5700, from mt5700webui-openwrt-server 3.0.2, folded in at
# build time by scripts/build-release.sh) is a pre-built React/Webpack SPA
# whose bundle already ships minified.  LuCI's build-time `jsmin` (Crockford
# JSMin) cannot parse regex literals that follow `=>`/`)`/`]` and rewrites
# them as division, which corrupts the bundle and white-screens the SPA at
# /5700/.  Disable JS minification for this package so the already-minified
# bundle ships untouched.  The Python AT backend (root/usr/bin/at-webserver.py)
# also requires python3 + websockets + pyserial (see LUCI_DEPENDS).
LUCI_MINIFY_JS:=0

PKG_NAME:=luci-app-mt5700m
PKG_VERSION:=2.3.41
PKG_RELEASE:=1
PKG_LICENSE:=Apache-2.0

LUCI_TITLE:=MT5700M module management
LUCI_DEPENDS:=+luci-base +ubus-at-daemon +sms-tool_q +python3 +python3-websockets +python3-pyserial
LUCI_EXTRA_DEPENDS:=kmod-usb2 (>=0), kmod-usb3 (>=0), kmod-usb-serial (>=0), kmod-usb-serial-option (>=0), kmod-usb-net (>=0), kmod-usb-net-cdc-ether (>=0), kmod-usb-net-cdc-ncm (>=0)
LUCI_PKGARCH:=all

define Package/luci-app-mt5700m/conffiles
/etc/mt5700m/traffic-history
endef

include $(TOPDIR)/feeds/luci/luci.mk

# call BuildPackage - OpenWrt buildroot signature
