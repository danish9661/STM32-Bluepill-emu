#!/bin/bash
# Live GDB session against the stub (dev-only: needs the Arduino toolchain's
# arm-none-eabi-gdb; NOT wired into CI). Proves real debuggers work, beyond
# the synthetic Node client in tests/test_gdbstub.mjs.
# Usage: ./tests/gdb_live_session.sh [port]
set -u
PORT=${1:-11223}
GDB=~/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-gdb
node -e "
import('./pkg/gdbstub.mjs').then(async g => {
  const fs = await import('fs');
  await g.serveGdb({ firmware: fs.readFileSync('site/arduino_echo.elf'), port: $PORT });
  setTimeout(() => process.exit(0), 120000).unref();
  await new Promise(() => {});
});" &
SRV=$!
sleep 3
timeout 90 "$GDB" -batch \
  -ex 'set pagination off' \
  -ex 'set confirm off' \
  -ex 'file site/arduino_echo.elf' \
  -ex "target remote :$PORT" \
  -ex 'info registers pc sp' \
  -ex 'x/2xw 0x08000000' \
  -ex 'break loop' \
  -ex 'continue' \
  -ex 'info registers pc' \
  -ex 'stepi' \
  -ex 'detach' \
  -ex 'quit'
RC=$?
kill $SRV 2>/dev/null
exit $RC
