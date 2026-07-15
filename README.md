# luci-app-mt5700m

面向 OpenWrt 与 QModem 的 MT5700M 5G 模组统一管理插件。它不嵌入或依赖模组 WebUI、`at-webserver`、Python 或 WebSocket 服务，设备信息与操作统一通过 QModem 配置和串行化 AT 通道完成。

## 功能

- 概况：连接状态、网络制式、信号、温度和设备信息
- 连接与拨号：复用 QModem 的 APN、协议、认证、路由优先级与重拨能力
- 网络与小区信息：服务小区、邻区、网络详情以及 LTE/NR 锁频
- 对话式短信、系统/FOTA、高级设置和独立 AT 控制台
- UCI 配置、LuCI 菜单、简体中文翻译以及最小化 rpcd ACL 均随包提供
- 自动模式优先使用 QModem 发现的串口与 `ubus-at-daemon`；串口服务不可用时才回退网络 AT

QModem 与本插件共享 `ubus-at-daemon` 串口仲裁，避免拨号监控和管理页面互相抢占 AT 端口。短信发送由 QModem 后端完成；请求一旦被受理便不会通过裸串口自动重试，避免重复发送。

## 依赖与兼容性

运行时依赖为 `luci-base`、BusyBox 和 QModem。插件仍保留串口与网络 TCP AT 的管理器配置，方便诊断和兼容特殊部署。

插件使用现代 LuCI JavaScript 视图和受限 rpcd `file.exec` ACL，适用于包含 LuCI、rpcd 与 QModem 的当前 OpenWrt 分支。没有设备专用 DTS、内核修改或防火墙规则。

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

1. QModem 能发现模组、读取 AT 端口并完成拨号。
2. 概况、连接、网络、小区、短信、系统、高级设置、AT 控制台与管理器设置均可打开。
3. 多页面并发查询通过 `ubus-at-daemon` 串行访问，不新增 QModem 串口冲突日志。
4. 模块返回 `ERROR` 时立即展示结果且不重复执行危险操作。
5. 非管理员 LuCI 会话只能使用本包 ACL 授权的 UCI、网络状态 RPC 与后端脚本。
6. `luci-i18n-mt5700m-zh-cn` 能安装且全部八个菜单页面显示正确。
