# luci-app-mt5700m

MT5700M 5G 模块的原生 LuCI 管理插件。它直接通过本机 AT 串口或模块的 TCP AT 端点工作，不嵌入、代理或依赖模块 WebUI、`at-webserver`、Python 或 WebSocket 服务。

## 功能

- 概览：模块型号、SIM、运营商、网络制式、信号、温度及锁频状态
- 网络与小区信息、LTE/NR 锁频、短信、系统/FOTA 和 AT 终端
- UCI 配置、LuCI 菜单、简体中文翻译以及最小化 rpcd ACL 均随包提供
- `auto` 模式先使用已配置或 QModem 发现的串口；串口不可用或读取超时才回退网络 AT

模块返回的 `OK` 和 `ERROR` 都是有效、即时返回的 AT 结果，不会触发网络重试。短信发送不会在串口已写入请求后自动重试，避免重复发送。

## 依赖与兼容性

运行时依赖为 `luci-base`、`rpcd-mod-file`、BusyBox 和 `netcat`。QModem 不是依赖：插件可以使用手工配置的串口，也可以仅配置网络 TCP AT 端点。

插件使用现代 LuCI JavaScript 视图和 rpcd `file.exec` ACL，适用于包含 LuCI 与 rpcd 的当前 OpenWrt 分支。没有设备专用 DTS、init 服务、防火墙规则或 WAN/LAN 假设。

## 作为 feed 使用

将本目录作为一个 feed 的包根目录。例如在目标 OpenWrt 构建树的 `feeds.conf.default` 中加入实际仓库地址：

```text
src-git mt5700m <你的-luci-app-mt5700m-feed-URL>
```

然后执行：

```sh
./scripts/feeds update mt5700m
./scripts/feeds install luci-app-mt5700m
make menuconfig
# LuCI -> Applications -> luci-app-mt5700m
```

也可以把目录直接复制到任意自定义 feed 后编译：

```sh
make package/luci-app-mt5700m/compile V=s
```

## UCI 配置

```uci
config mt5700m 'settings'
	option enabled '1'
	option mode 'auto'       # auto | serial | network
	option at_port ''        # 例如 /dev/ttyUSB1；留空时尝试 QModem 配置
	option host '192.168.8.1'
	option port '20249'
	option timeout '8'
```

网络模式会依次尝试自动检测到的蜂窝网关、配置主机、`192.168.8.1` 和 `10.0.0.1`。串口模式仅使用配置串口或 QModem 声明的 `at_port`。

## 安全边界

LuCI 前端只能经 rpcd 执行 `/usr/sbin/mt5700m-at`，并且仅可读写 `mt5700m` UCI 配置。脚本会清除 AT 命令中的换行和 NUL；短信号码与删除索引也会被约束。FOTA、重启、短信和锁频都是高风险操作，页面保留明确确认提示。

## 验证清单

在目标设备或镜像测试中至少覆盖：

1. 不安装 `at-webserver` 与 QModem 时，手工网络 AT 配置可工作。
2. 仅串口、仅网络及 `auto` 三种模式。
3. `auto` 串口成功、串口不存在、串口超时后网络回退。
4. 模块返回 `ERROR` 时立即展示结果且不重复发送。
5. 非管理员 LuCI 会话只能使用本包 ACL 授权的 UCI 与后端脚本。
6. `luci-i18n-mt5700m-zh-cn` 能安装且所有七个菜单页面可打开。
