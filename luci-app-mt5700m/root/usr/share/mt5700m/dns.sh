#!/bin/sh
# DNS 应用已移植到 Rust/Tokio 后端（at-webserver dns ...）。
# 保留这个入口只是为了让 hotplug / init.d / 运维脚本的路径不变。
PROG=/usr/bin/at-webserver

[ -x "$PROG" ] || {
	echo "dns.sh: $PROG not installed" >&2
	exit 1
}

exec "$PROG" dns "$@"
