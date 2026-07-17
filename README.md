# MT5700M Manager for OpenWrt

[![CI](https://github.com/FAN789/luci-app-mt5700m/actions/workflows/ci.yml/badge.svg)](https://github.com/FAN789/luci-app-mt5700m/actions/workflows/ci.yml)
[![Build Release](https://github.com/FAN789/luci-app-mt5700m/actions/workflows/release.yml/badge.svg)](https://github.com/FAN789/luci-app-mt5700m/actions/workflows/release.yml)

专门面向移远 MT5700M-CN 5G 模组的 OpenWrt LuCI 管理器。它把状态、移动
数据、网络与小区、短信、系统维护和 AT 终端统一到一个应用中，并按照 MT5700M
手册识别 USB 正常、升级和 Dump 模式。

## 主要功能

- 模组、SIM、运营商、IPv4/IPv6、温度和载波聚合概况
- 首页统一展示网络会话、模组原生计数和本机日/月流量历史
- RSRP/RSRQ/SINR 等信号质量的人性化显示
- APN、PDP、漫游和移动数据连接管理
- 网络制式、LTE/WCDMA 频段及小区锁定
- 短信收发与联系人友好的列表界面
- IMEI、手机号、签约速率、USB/网口状态及系统维护
- MT5700M 专用 AT 终端和 USB 接口自动探测
- 简体中文界面；不依赖云服务，不上传模组或 SIM 数据

流量历史由应用直接读取 MT5700M 数据接口的内核计数，不依赖 `vnStat`，并保存在
`/etc/mt5700m/traffic-history`。升级自早期独立流量插件时会自动迁移已有记录；不再
安装或显示单独的“流量统计”应用。

## 安装和编译

源码包位于 `luci-app-mt5700m/`：

```sh
git clone https://github.com/FAN789/luci-app-mt5700m.git
cp -a luci-app-mt5700m/luci-app-mt5700m /path/to/openwrt/package/
make menuconfig
# LuCI -> Applications -> luci-app-mt5700m
make package/luci-app-mt5700m/compile V=s
```

应用依赖 `ubus-at-daemon`、`sms-tool_q` 及 OpenWrt 官方 USB 串口/网卡内核模块。
每个 GitHub Release 均由 GitHub Actions 使用官方 OpenWrt SNAPSHOT
`mediatek/filogic` SDK 在线构建，附带应用、中文包、两个底层传输包、SDK 构建公钥
和 SHA256 校验文件。安装时必须使用与设备固件 ABI/内核版本相匹配的软件源。

## 设计边界

本项目不是通用蜂窝模组框架，只实现 MT5700M 所需的能力。低层 AT 与短信传输
包来自固定版本的 [FUjr/QModem](https://github.com/FUjr/QModem)，应用内的精简
实现保留了来源说明，详见
[`QMODEM-NOTICE`](luci-app-mt5700m/root/usr/share/mt5700m/QMODEM-NOTICE)。
该部分受其上游 MPL-2.0 和非商业限制约束，适用于个人、非商业用途。

本仓库自行编写的代码按 [Apache License 2.0](LICENSE) 发布。
