# Changelog

## [2.3.33] - 2026-08-18

### Fixed
- 修复空 APN 时自动拨号拼接空字段导致模组返回 ERROR、dial_mode 静默保持旧值的问题。空 APN 时改用 `AT^SETAUTODIAL=1,1,"IPV4V6"`（省略空字段），USB 数传拨号模式可正常生效。

## [2.3.32] - 2026-08-17

### Fixed
- 修复 init.d/at-webserver 缺少执行位导致 procd 跳过、8765 端口无监听、实时面板全零的问题。
