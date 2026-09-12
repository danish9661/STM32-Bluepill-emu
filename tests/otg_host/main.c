/* Bare-metal USB OTG FS host (HCD) for STM32F105: enumerates a virtual
 * device and runs a bulk echo, proving the emulator's OTG_FS host model
 * through real machine code. Polled (no NVIC): port reset, control
 * transfers (SETUP + IN + status OUT) on channel 0, bulk OUT/IN on
 * channels 1/2. The peer device is scripted host-side (see
 * tests/test_otg_host.mjs, which answers HostTx/HostRx events).
 * Progress is recorded in `trace[]` (RAM) for exact-path assertions.
 * Built with xpack gcc (see build.sh); the ELF ships at
 * site/otg_host.elf for CI.
 */
typedef unsigned int uint32_t;
typedef unsigned char uint8_t;

#define USB   0x50000000u
#define RCC_AHBENR 0x4002101Cu
#define reg(a) (*(volatile uint32_t *)(a))

#define GAHBCFG 0x008
#define GRSTCTL 0x010
#define GINTSTS 0x014
#define GRXSTSP 0x020
#define GRXFSIZ 0x024
#define GNPTXFSIZ 0x028
#define HCFG 0x400
#define HFNUM 0x408
#define HPRT 0x440
#define DCFG_ALIAS 0x800 /* untouched in host mode (readback sanity) */

#define CHENA (1u << 31)
#define CHDIS (1u << 30)
#define EPDIR_IN (1u << 15)
#define XFRC 1u
#define CHHLT 2u
#define STALL 8u

#define PID_DATA0 0u
#define PID_DATA1 1u
#define PID_SETUP 3u

#define trace_push(c) do { if (trace_n < 32) trace[trace_n++] = (c); } while (0)
/* Volatile: main never returns, so gcc -Os would otherwise keep trace_n
 * in a register across the bulk block and the host could never observe
 * pushes 6..8 (stores land, the increment never flushes). */
static volatile uint32_t trace[32];
static volatile uint32_t trace_n = 0;
static uint8_t xfer_buf[128];

static void fifo_write(uint32_t fifo, const uint8_t *src, uint32_t len) {
    uint32_t n = (len + 3) >> 2, i, j;
    for (i = 0; i < n; i++) {
        uint32_t w = 0;
        for (j = 0; j < 4; j++) {
            uint32_t k = i * 4 + j;
            if (k < len) w |= (uint32_t)src[k] << (j * 8);
        }
        reg(USB + fifo + i * 4) = w;
    }
}

static uint32_t hcchar(uint32_t ch) { return USB + 0x500 + ch * 0x20; }
static uint32_t hcint(uint32_t ch) { return USB + 0x508 + ch * 0x20; }
static uint32_t hctsiz(uint32_t ch) { return USB + 0x510 + ch * 0x20; }
static uint32_t dfifo(uint32_t ch) { return 0x1000 + ch * 0x1000; }

static int wait_hcint(uint32_t ch, uint32_t mask) {
    uint32_t t = 0;
    while (!(reg(hcint(ch)) & mask)) {
        if (++t > 50000000u) return -1;
    }
    return 0;
}

/* OUT/SETUP transfer on ch: program, enable, push FIFO, wait XFRC. */
static int hc_out(uint32_t ch, uint32_t ep, uint32_t type, uint32_t dpid,
                  const uint8_t *src, uint32_t len) {
    uint32_t pkts = len == 0 ? 1u : (len + 63) >> 6;
    reg(hctsiz(ch)) = len | (pkts << 19) | (dpid << 29);
    reg(hcchar(ch)) = 64u | (ep << 11) | (type << 18) | CHENA;
    if (len) fifo_write(dfifo(ch), src, len);
    if (wait_hcint(ch, XFRC)) return -1;
    reg(hcint(ch)) = XFRC;
    return 0;
}

/* IN transfer on ch: program, enable, wait XFRC, drain RXFIFO by BCNT.
 * Drain-first order: GRXSTSP statuses are consumed before XFRC is
 * honored, so a completion that lands (fed/silicon) before the first
 * poll still delivers its bytes instead of returning 0. Only data
 * statuses (PKTSTS 2) carry new FIFO bytes — the transfer-completed
 * status drains nothing, like ST's HCD. The true byte count comes
 * from HCTSIZ.XFRSIZ (initial minus remaining), which also drops the
 * FIFO word-padding tail on non-multiple-of-4 lengths.
 * Complete-then-recheck: after XFRC is observed, drain again to
 * empty. XFRC can only be set after the feed/transfer, so every load
 * past that point is live — a status snapshot racing the completion
 * (batch/interrupt boundary between the GRXSTSP load and its use)
 * can never hide bytes here. Without the recheck, an
 * "empty (stale) + complete (live)" split exits with got=0. */
static int hc_in(uint32_t ch, uint32_t ep, uint32_t type, uint32_t dpid,
                 uint8_t *dst, uint32_t len) {
    uint32_t pkts = len == 0 ? 1u : (len + 63) >> 6;
    uint32_t xfr = pkts * 64u;
    uint32_t got = 0, t = 0;
    reg(hctsiz(ch)) = xfr | (pkts << 19) | (dpid << 29);
    reg(hcchar(ch)) = 64u | (ep << 11) | EPDIR_IN | (type << 18) | CHENA;
    /* Drain every GRXSTSP status until our XFRC lands with none left. */
    for (;;) {
        if (reg(USB + GINTSTS) & (1u << 4)) {
            uint32_t st = reg(USB + GRXSTSP);
            uint32_t pkt = (st >> 17) & 0xF;
            uint32_t bcnt = (st >> 4) & 0x7FF;
            uint32_t words = (bcnt + 3) >> 2, i;
            if (pkt == 2) {
                for (i = 0; i < words && got < len; i++) {
                    uint32_t w = reg(USB + 0x1000);
                    uint32_t k;
                    for (k = 0; k < 4 && got < len; k++)
                        dst[got++] = (uint8_t)(w >> (k * 8));
                }
            }
            t = 0;
            continue;
        }
        if (reg(hcint(ch)) & XFRC) break;
        if (++t > 50000000u) return -1;
    }
    /* Recheck: statuses queued with the completion are still level-
     * present; pop them all (data ones into the tail of dst). */
    while (reg(USB + GINTSTS) & (1u << 4)) {
        uint32_t st = reg(USB + GRXSTSP);
        uint32_t pkt = (st >> 17) & 0xF;
        uint32_t bcnt = (st >> 4) & 0x7FF;
        uint32_t words = (bcnt + 3) >> 2, i;
        if (pkt == 2) {
            for (i = 0; i < words && got < len; i++) {
                uint32_t w = reg(USB + 0x1000);
                uint32_t k;
                for (k = 0; k < 4 && got < len; k++)
                    dst[got++] = (uint8_t)(w >> (k * 8));
            }
        }
    }
    reg(hcint(ch)) = XFRC;
    return (int)(xfr - (reg(hctsiz(ch)) & 0x7FFFFu));
}

/* SETUP stage (8 bytes, PID SETUP) on channel 0. */
static int setup_stage(const uint8_t *s) {
    return hc_out(0, 0, 0, PID_SETUP, s, 8);
}

static const uint8_t S_DEV[8] = {0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00};
static const uint8_t S_ADDR[8] = {0x00, 0x05, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00};
static const uint8_t S_CFG[8] = {0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 0xFF, 0x00};
static const uint8_t S_SETCFG[8] = {0x00, 0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00};

int main(void) {
    uint32_t i, n;
    reg(RCC_AHBENR) |= (1u << 12); /* OTGFSEN */
    reg(USB + GAHBCFG) |= 1u; /* GINT */
    reg(USB + GRSTCTL) = 1u; /* CSFTRST */
    while (reg(USB + GRSTCTL) & 1u) { }
    reg(USB + GRXFSIZ) = 128u;
    reg(USB + GNPTXFSIZ) = (64u << 16) | 128u;
    reg(USB + HCFG) = 0;
    reg(USB + HPRT) |= (1u << 12); /* PPWR */
    /* Wait for a device (test attaches one), then bus-reset it. */
    i = 0;
    while (!(reg(USB + HPRT) & 1u)) {
        if (++i > 100000000u) { trace_push(0xE0); for (;;) { } }
    }
    reg(USB + HPRT) |= (1u << 8); /* PRST */
    for (i = 0; i < 2000000u; i++) { }
    reg(USB + HPRT) &= ~(1u << 8);
    trace_push(1);

    /* 1. GET_DESCRIPTOR device (18B): SETUP + IN DATA1 + status OUT. */
    if (setup_stage(S_DEV)) { trace_push(0xE1); for (;;) { } }
    n = hc_in(0, 0, 0, PID_DATA1, xfer_buf, 18);
    if (n != 18 || xfer_buf[0] != 18 || xfer_buf[1] != 1) { trace_push(0xE2); for (;;) { } }
    if (hc_out(0, 0, 0, PID_DATA1, 0, 0)) { trace_push(0xE3); for (;;) { } }
    trace_push(2);

    /* 2. SET_ADDRESS(5) + status IN. */
    if (setup_stage(S_ADDR)) { trace_push(0xE4); for (;;) { } }
    if (hc_in(0, 0, 0, PID_DATA1, xfer_buf, 0) != 0) { trace_push(0xE5); for (;;) { } }
    trace_push(3);

    /* 3. GET_DESCRIPTOR config (ask 255, expect 75B: 64 + 11). */
    if (setup_stage(S_CFG)) { trace_push(0xE6); for (;;) { } }
    n = hc_in(0, 0, 0, PID_DATA1, xfer_buf, 255);
    if (n != 75 || xfer_buf[0] != 9 || xfer_buf[1] != 2 || xfer_buf[2] != 75) { trace_push(0xE7); for (;;) { } }
    if (hc_out(0, 0, 0, PID_DATA1, 0, 0)) { trace_push(0xE8); for (;;) { } }
    trace_push(4);

    /* 4. SET_CONFIGURATION(1) + status IN. */
    if (setup_stage(S_SETCFG)) { trace_push(0xE9); for (;;) { } }
    if (hc_in(0, 0, 0, PID_DATA1, xfer_buf, 0) != 0) { trace_push(0xEA); for (;;) { } }
    trace_push(5);

    /* 5. Bulk OUT EP1 "Hi" on ch1, bulk IN EP1 on ch2, verify echo. */
    {
        const uint8_t hi[2] = {'H', 'i'};
        if (hc_out(1, 1, 2, PID_DATA0, hi, 2)) { trace_push(0xEB); for (;;) { } }
        trace_push(6);
        n = hc_in(2, 1, 2, PID_DATA1, xfer_buf, 2);
        if (n != 2 || xfer_buf[0] != 'H' || xfer_buf[1] != 'i') { trace_push(0xEC); for (;;) { } }
        trace_push(7);
    }
    trace_push(8);
    for (;;) { }
    return 0;
}

/* --- Startup: vector table + .data/.bss init, then main --- */
extern uint32_t _estack, _sidata, _sdata, _edata, _sbss, _ebss;
void Reset_Handler(void) {
    uint32_t *s = &_sidata, *d = &_sdata;
    while (d < &_edata) *d++ = *s++;
    d = &_sbss;
    while (d < &_ebss) *d++ = 0;
    main();
    for (;;) { }
}
void Default_Handler(void) { for (;;) { } }
__attribute__((section(".isr_vector")))
uint32_t *vectors[] = {
    (uint32_t *)&_estack, (uint32_t *)Reset_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
    (uint32_t *)Default_Handler, (uint32_t *)Default_Handler,
};
