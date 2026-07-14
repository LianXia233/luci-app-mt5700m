include $(TOPDIR)/rules.mk

PKG_NAME:=luci-app-mt5700m
PKG_VERSION:=1.2.0
PKG_RELEASE:=4

LUCI_TITLE:=MT5700M modem management
LUCI_DEPENDS:=+luci-base +rpcd-mod-file +busybox +netcat +ubus-at-daemon +jsonfilter +libubox
LUCI_PKGARCH:=all

include $(TOPDIR)/feeds/luci/luci.mk

# call BuildPackage - OpenWrt buildroot signature
