include $(TOPDIR)/rules.mk

PKG_NAME:=luci-app-mt5700m
PKG_VERSION:=2.2.1
PKG_RELEASE:=1
PKG_LICENSE:=Apache-2.0

LUCI_TITLE:=MT5700M Module Management
LUCI_PKGARCH:=all

# 编译依赖
LUCI_DEPENDS:= \
	+luci-base \
	+ubus-at-daemon \
	+sms-tool_q

define Package/luci-app-mt5700m/conffiles
/etc/mt5700m/traffic-history
endef

include $(TOPDIR)/feeds/luci/luci.mk

# call BuildPackage - OpenWrt buildroot signature
