include $(TOPDIR)/rules.mk

PKG_NAME:=at-webserver
PKG_VERSION:=4.0.0
PKG_RELEASE:=1

PKG_LICENSE:=MIT
PKG_BUILD_PARALLEL:=1
PKG_BUILD_FLAGS:=no-mips16

# Rust backend: single std-only binary (no third-party crates, no cargo
# index / vendored sources needed). Feed builds with the packages feed's
# lang/rust compile from source; SDK/CI builds (scripts/build-release.sh)
# inject a prebuilt musl binary into ./prebuilt/ instead and skip the
# compile step entirely.
ifdef CONFIG_PACKAGE_at-webserver
RUST_PACKAGE_MK:=$(wildcard $(TOPDIR)/feeds/packages/lang/rust/rust-package.mk)
endif

include $(INCLUDE_DIR)/package.mk
ifneq ($(strip $(RUST_PACKAGE_MK)),)
include $(RUST_PACKAGE_MK)
endif

define Package/at-webserver
  SECTION:=net
  CATEGORY:=Network
  TITLE:=MT5700M AT WebSocket daemon (Rust, dual frontend)
  URL:=https://github.com/LianXia233/luci-app-mt5700m
  DEPENDS:=@(aarch64||arm||x86_64||i386||mips||mipsel)
endef

define Package/at-webserver/description
  WebSocket server bridging AT commands to the MT5700 WebUI, plus the LuCI
  shell backend (mt5700m-at) via argv[0] dispatch on the same binary.
  UBUS transport (via ubus-at-daemon) by default; SERIAL/NETWORK optional.
  Static Rust binary, no Python runtime required.
endef

define Build/Prepare
	mkdir -p $(PKG_BUILD_DIR)
	$(CP) ./Cargo.toml ./Cargo.lock $(PKG_BUILD_DIR)/
	$(CP) ./src $(PKG_BUILD_DIR)/src
endef

define Build/Compile
ifdef RUST_PACKAGE_MK
	+$(CARGO_PKG_VARS) $(CARGO_PKG_ARGS) cargo build --release --offline
else
	echo "ERROR: packages feed lang/rust not found. Enable the packages feed or inject a prebuilt binary." >&2
	exit 1
endif
endef

define Package/at-webserver/install
	$(INSTALL_DIR) $(1)/usr/bin
	$(INSTALL_BIN) $(PKG_BUILD_DIR)/target/release/at-webserver $(1)/usr/bin/at-webserver

	$(INSTALL_DIR) $(1)/etc/init.d
	$(INSTALL_BIN) ./files/etc/init.d/at-webserver $(1)/etc/init.d/

	$(INSTALL_DIR) $(1)/etc/config
	$(INSTALL_CONF) ./files/etc/config/at-webserver $(1)/etc/config/

	$(INSTALL_DIR) $(1)/www/5700
	$(CP) -r ./files/www/5700/* $(1)/www/5700/

	$(INSTALL_DIR) $(1)/www/cgi-bin
	$(INSTALL_BIN) ./files/www/cgi-bin/at-ws-info $(1)/www/cgi-bin/
	$(INSTALL_BIN) ./files/www/cgi-bin/at-log-clear $(1)/www/cgi-bin/
endef

define Package/at-webserver/conffiles
/etc/config/at-webserver
endef

# 从 2.x/3.x 的 Python 版升级时清掉旧脚本，避免 /usr/bin 里留个死文件。
define Package/at-webserver/preinst
#!/bin/sh
rm -f /usr/bin/at-server.py /usr/bin/at-webserver.py
exit 0
endef

$(eval $(call BuildPackage,at-webserver))
