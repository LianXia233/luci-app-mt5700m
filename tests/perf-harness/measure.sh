#!/usr/bin/env bash
# Collect N perf runs for one variant via playwright-cli.
# Usage: measure.sh <playwright_cli.sh> <before|after> [runs]
set -u
PW="$1"
VARIANT="$2"
RUNS="${3:-5}"

cli() { bash "$PW" "$@" 2>&1; }

# print the line following "### Result"
result_line() {
	awk '/^### Result$/{getline; print; exit}'
}

for i in $(seq 1 "$RUNS"); do
	if [ "$i" -eq 1 ]; then
		cli open "http://127.0.0.1:8901/$VARIANT/" >/dev/null
	else
		cli reload >/dev/null
	fi
	ok=""
	for _t in $(seq 1 80); do
		v="$(cli eval 'window.__perf && window.__perf.done' | result_line)"
		case "$v" in
			true) ok=1; break ;;
		esac
		sleep 0.2
	done
	if [ -z "$ok" ]; then
		echo "{\"variant\":\"$VARIANT\",\"run\":$i,\"error\":\"timeout\"}"
		continue
	fi
	cli eval 'JSON.stringify(window.__perf)' | result_line
done
