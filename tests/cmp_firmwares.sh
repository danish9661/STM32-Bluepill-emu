#!/usr/bin/env bash
# Compare Path A (merged wasm TCI) vs dual-module (emulator.js wasm TCI) across
# multiple firmwares. Checks: (1) identical UART output (correctness), (2) speed.
set -u
ROOT="/home/danish1075/Documents/stm32 emu blue pill"
cd "$ROOT"
MAX=200000000
printf 'AB' > /tmp/stdin.bin
printf 'hello world\n' > /tmp/echo_in.txt

# firmware:config:stdinfile
CASES=(
  "arduino_echo:/tmp/echo_in.txt"
  "arduino_timer_uart:"
  "arduino_adc_uart:"
  "arduino_fade:"
  "arduino_ws2812:"
  "arduino_periph_test:/tmp/stdin.bin"
)

echo "firmware                | PathA MIPS | Dual MIPS | UART match"
echo "------------------------|------------|-----------|-----------"
for c in "${CASES[@]}"; do
  fw="${c%%:*}"; stdin="${c##*:}"
  cfg="tests/$fw/config.yaml"
  pa_args=(pkg/bench_merged.mjs --config="$cfg" --max=$MAX --uart-out=/tmp/pa_$fw.uart)
  du_args=(pkg/bench_dual.mjs   --config="$cfg" --max=$MAX --uart-out=/tmp/dual_$fw.uart)
  [ -n "$stdin" ] && { pa_args+=(--stdin-file="$stdin"); du_args+=(--stdin-file="$stdin"); }

  node "${pa_args[@]}" >/tmp/pa_$fw.log 2>&1
  node "${du_args[@]}" >/tmp/dual_$fw.log 2>&1

  pa_mips=$(grep -oE 'instructions in [0-9.]+s' /tmp/pa_$fw.log | grep -oE '[0-9.]+s' | tr -d 's')
  du_mips=$(grep -oE 'instructions in [0-9.]+s' /tmp/dual_$fw.log | grep -oE '[0-9.]+s' | tr -d 's')
  pa_rate=$(awk "BEGIN{if($pa_mips>0) printf \"%.1f\", $MAX/$pa_mips/1e6; else print \"?\"}")
  du_rate=$(awk "BEGIN{if($du_mips>0) printf \"%.1f\", $MAX/$du_mips/1e6; else print \"?\"}")

  if diff -q /tmp/pa_$fw.uart /tmp/dual_$fw.uart >/dev/null 2>&1; then match="MATCH"; else match="DIFFER"; fi
  printf "%-23s | %10s | %9s | %s\n" "$fw" "$pa_rate" "$du_rate" "$match"
done
