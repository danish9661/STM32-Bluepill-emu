/* Bare-metal USB OTG FS CDC-ACM device for STM32F105 (register-level, no
 * HAL, no libc): proves the emulator's OTG_FS device model through real
 * machine code. Polled (no NVIC): answers host enumeration (device /
 * configuration / string descriptors, SET_ADDRESS, SET_CONFIGURATION, CDC
 * SET_LINE_CODING + SET_CONTROL_LINE_STATE) then echoes bulk traffic on
 * EP1 (OUT -> IN loopback). Drive it headless with otg_inject_setup /
 * otg_inject_out and read the replies from UsbIn events (see
 * tests/test_otg_cdc.mjs). Built with xpack gcc (see build.sh); the ELF
 * ships at site/otg_cdc.elf for CI.
 */
typedef unsigned int uint32_t;
typedef unsigned char uint8_t;

#define USB   0x50000000u
#define RCC_AHBENR 0x4002101Cu
#define reg(a) (*(volatile uint32_t *)(a))

#define GOTGCTL 0x000
#define GAHBCFG 0x008
#define GRSTCTL 0x010
#define GINTSTS 0x014
#define GINTMSK 0x018
#define GRXSTSP 0x020
#define GRXFSIZ 0x024
#define GNPTXFSIZ 0x028
#define GCCFG 0x038
#define DIEPTXF1 0x104
#define DCFG 0x800
#define DCTL 0x804
#define DIEPMSK 0x810
#define DOEPMSK 0x814
#define DAINT 0x818
#define DAINTMSK 0x81C
#define DIEPCTL0 0x900
#define DIEPINT0 0x908
#define DIEPTSIZ0 0x910
#define DIEPCTL1 0x920
#define DIEPINT1 0x928
#define DIEPTSIZ1 0x930
#define DIEPCTL3 0x960
#define DIEPINT3 0x968
#define DIEPTSIZ3 0x970
#define DOEPCTL0 0xB00
#define DOEPINT0 0xB08
#define DOEPTSIZ0 0xB10
#define DOEPCTL1 0xB20
#define DOEPINT1 0xB28
#define DOEPTSIZ1 0xB30
#define DFIFO0 0x1000
#define DFIFO1 0x2000

#define EPENA (1u << 31)
#define EPDIS (1u << 30)
#define SNAK (1u << 27)
#define CNAK (1u << 26)
#define STALL (1u << 21)
#define USBAEP (1u << 15)
#define TXFNUM1 (1u << 22)
#define TXFNUM3 (3u << 22)
#define XFRC 1u
#define EPDISD 2u
#define STUP 8u

#define G_USBRST (1u << 12)
#define G_ENUMDNE (1u << 13)
#define G_RXFLVL (1u << 4)
#define G_IEPINT (1u << 18)
#define G_OEPINT (1u << 19)

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

/* --- Descriptors (same shapes as the FS-device CDC demo) --- */
static const uint8_t DEV_DESC[] = {
    18, 1, 0x00, 0x02, 0xEF, 0x02, 0x01, 64,
    0x83, 0x04, 0x40, 0x57, 0x00, 0x01, 1, 2, 0, 1,
};
static const uint8_t CFG_DESC[] = {
    9, 2, 75, 0, 2, 1, 0, 0x80, 100,
    8, 11, 0, 2, 0x02, 0x02, 0x01, 0,
    9, 4, 0, 0, 1, 0x02, 0x02, 0x01, 0,
    5, 0x24, 0x00, 0x10, 0x01,
    5, 0x24, 0x01, 0x00, 1,
    4, 0x24, 0x02, 0x02,
    5, 0x24, 0x06, 0, 1,
    7, 5, 0x83, 0x03, 8, 0, 10,
    9, 4, 1, 0, 2, 0x0A, 0x00, 0x00, 0,
    7, 5, 0x01, 0x02, 64, 0, 0,
    7, 5, 0x81, 0x02, 64, 0, 0,
};
static const uint8_t STR0_DESC[] = { 4, 3, 0x09, 0x04 };

/* --- EP0 control state --- */
enum Ep0State { IDLE, IN_STATUS, OUT_DATA };
static enum Ep0State ep0state = IDLE;
static uint8_t out_stage[64];

/* Single-stage: the whole transfer (even multi-packet sizes) is pushed
 * up front and completes once; no per-packet continuation needed. */
static void ep0_tx(const uint8_t *src, uint32_t len) {
    reg(USB + DIEPTSIZ0) = (2u << 19) | len;
    reg(USB + DIEPCTL0) |= EPENA;
    fifo_write(DFIFO0, src, len);
}
static void ep0_tx_status(void) {
    reg(USB + DIEPTSIZ0) = (1u << 19);
    reg(USB + DIEPCTL0) |= EPENA;
}
static void ep0_rx_arm(uint32_t len) {
    reg(USB + DOEPTSIZ0) = (1u << 19) | len;
    reg(USB + DOEPCTL0) |= EPENA | CNAK;
}
static void ep0_rx_setup_arm(void) {
    reg(USB + DOEPTSIZ0) = (3u << 29) | (1u << 19) | 8u;
    reg(USB + DOEPCTL0) |= EPENA | CNAK;
}

static void handle_setup(void) {
    uint8_t s[8];
    uint32_t w0 = reg(USB + DFIFO0);
    uint32_t w1 = reg(USB + DFIFO0);
    s[0] = w0; s[1] = w0 >> 8; s[2] = w0 >> 16; s[3] = w0 >> 24;
    s[4] = w1; s[5] = w1 >> 8; s[6] = w1 >> 16; s[7] = w1 >> 24;
    /* Retire the SETUP event first so a new one can land. */
    reg(USB + DOEPINT0) = XFRC | STUP;
    ep0_rx_setup_arm();
    {
        uint8_t req_type = s[0], req = s[1];
        uint32_t wValue = s[2] | ((uint32_t)s[3] << 8);
        uint32_t wLength = s[6] | ((uint32_t)s[7] << 8);
        if (req_type == 0x80 && req == 0x06) {
            const uint8_t *d = 0;
            uint32_t n = 0;
            uint32_t dtype = wValue >> 8, didx = wValue & 0xFF;
            if (dtype == 1 && didx == 0) { d = DEV_DESC; n = sizeof(DEV_DESC); }
            else if (dtype == 2 && didx == 0) { d = CFG_DESC; n = sizeof(CFG_DESC); }
            else if (dtype == 3 && didx == 0) { d = STR0_DESC; n = sizeof(STR0_DESC); }
            if (d && n) {
                if (n > wLength) n = wLength;
                ep0_tx(d, n);
                ep0state = IN_STATUS;
                return;
            }
        } else if (req_type == 0x00 && req == 0x05) {
            reg(USB + DCFG) = (reg(USB + DCFG) & ~0x7F0u) | (((uint32_t)(wValue & 0x7F)) << 4);
            ep0_tx_status();
            ep0state = IN_STATUS;
            return;
        } else if (req_type == 0x00 && req == 0x09) {
            /* Open EP1 bulk OUT/IN + EP3 interrupt IN. */
            reg(USB + DOEPCTL1) = USBAEP;
            reg(USB + DIEPCTL1) = USBAEP | TXFNUM1 | 64u;
            reg(USB + DIEPCTL3) = USBAEP | TXFNUM3 | 8u;
            reg(USB + DAINTMSK) |= (1u << 1) | (1u << 17) | (1u << 3);
            reg(USB + DOEPTSIZ1) = (1u << 19) | 64u;
            reg(USB + DOEPCTL1) |= EPENA | CNAK;
            ep0_tx_status();
            ep0state = IN_STATUS;
            return;
        } else if (req_type == 0x21 && req == 0x20) {
            ep0state = OUT_DATA;
            /* Arm the 7-byte OUT data stage (overrides the setup re-arm). */
            reg(USB + DOEPTSIZ0) = (1u << 19) | 7u;
            reg(USB + DOEPCTL0) |= EPENA | CNAK;
            return;
        } else if (req_type == 0x21 && req == 0x22) {
            ep0_tx_status();
            ep0state = IN_STATUS;
            return;
        }
        reg(USB + DIEPCTL0) |= STALL;
        reg(USB + DOEPCTL0) |= STALL;
        ep0state = IDLE;
    }
}

static void handle_out_done(uint32_t ep, uint32_t bcnt) {
    (void)bcnt;
    reg(USB + (ep == 0 ? DOEPINT0 : DOEPINT1)) = XFRC;
    if (ep == 0) {
        if (ep0state == OUT_DATA) {
            /* Line-coding bytes already sit in out_stage; status stage. */
            ep0_tx_status();
            ep0state = IN_STATUS;
        } else {
            ep0state = IDLE;
        }
        ep0_rx_setup_arm();
    } else if (ep == 1) {
        /* Bulk echo: IN mirrors the OUT just received. */
        uint32_t n = bcnt > 64 ? 64 : bcnt;
        reg(USB + DIEPTSIZ1) = (1u << 19) | n;
        reg(USB + DIEPCTL1) |= EPENA;
        fifo_write(DFIFO1, out_stage, n);
        reg(USB + DOEPTSIZ1) = (1u << 19) | 64u;
        reg(USB + DOEPCTL1) |= EPENA | CNAK;
    }
}

static void on_rx(void) {
    uint32_t st = reg(USB + GRXSTSP);
    uint32_t ep = st & 0xF;
    uint32_t bcnt = (st >> 4) & 0x7FF;
    uint32_t pkt = (st >> 17) & 0xF;
    uint32_t i, w;
    if (pkt == 6 || pkt == 4) {
        if (pkt == 4) handle_setup();
        return; /* SETUP bytes are pulled inside handle_setup. */
    }
    if (ep == 0 || ep == 1) {
        /* Read packet bytes only on the received status; the completed
         * status carries the same BCNT but the FIFO was already drained. */
        if (pkt == 2) {
            uint32_t words = (bcnt + 3) >> 2;
            uint32_t i, w;
            for (i = 0; i < words; i++) {
                w = reg(USB + DFIFO0);
                {
                    uint32_t k;
                    for (k = 0; k < 4; k++) {
                        uint32_t idx = i * 4 + k;
                        if (idx < bcnt && idx < 64) out_stage[idx] = (uint8_t)(w >> (k * 8));
                    }
                }
            }
        }
    }
    if (pkt == 3) handle_out_done(ep, bcnt);
}

static void on_in(void) {
    uint32_t da = reg(USB + DAINT);
    uint32_t e;
    for (e = 0; e < 4; e++) {
        if (da & (1u << e)) {
            uint32_t fl = reg(USB + (e == 0 ? DIEPINT0 : e == 1 ? DIEPINT1 : DIEPINT3));
            if (fl & XFRC) {
                reg(USB + (e == 0 ? DIEPINT0 : e == 1 ? DIEPINT1 : DIEPINT3)) = XFRC;
                if (e == 0) {
                    ep0state = IDLE;
                    /* Arm the status-OUT stage now that IN is done. */
                    reg(USB + DOEPTSIZ0) = (1u << 19);
                    reg(USB + DOEPCTL0) |= EPENA | CNAK;
                }
            }
        }
    }
}

static void on_out_events(void) {
    uint32_t da = reg(USB + DAINT);
    uint32_t e;
    for (e = 0; e < 4; e++) {
        if (da & (1u << (16 + e))) {
            uint32_t r = e == 0 ? DOEPINT0 : DOEPINT1;
            reg(USB + r) = reg(USB + r); /* W1C retire (XFRC etc already consumed). */
        }
    }
}

static void on_reset(void) {
    /* Open EP0 like ST's Reset handler (IN MPS 64, OUT armed for SETUP). */
    reg(USB + DIEPCTL0) = USBAEP;
    reg(USB + DOEPCTL0) = USBAEP;
    reg(USB + DOEPTSIZ0) = (3u << 29) | (1u << 19) | 8u;
    reg(USB + DOEPCTL0) |= EPENA | CNAK;
    reg(USB + DAINTMSK) = (1u << 0) | (1u << 16);
    ep0state = IDLE;
}

int main(void) {
    reg(RCC_AHBENR) |= (1u << 12); /* OTGFSEN */
    reg(USB + GAHBCFG) |= 1u; /* GINT: global interrupt line */
    reg(USB + GRSTCTL) = 1u; /* CSFTRST */
    while (reg(USB + GRSTCTL) & 1u) { }
    reg(USB + GRXFSIZ) = 128u;
    reg(USB + GNPTXFSIZ) = (64u << 16) | 128u;
    reg(USB + DIEPTXF1) = (64u << 16) | 192u;
    reg(USB + GINTMSK) = G_USBRST | G_ENUMDNE | G_RXFLVL | G_IEPINT | G_OEPINT;
    reg(USB + DIEPMSK) = 1u;
    reg(USB + DOEPMSK) = 1u | 8u;
    for (;;) {
        uint32_t g = reg(USB + GINTSTS);
        if (g & G_USBRST) {
            on_reset();
            reg(USB + GINTSTS) = G_USBRST | G_ENUMDNE;
        }
        if (g & G_RXFLVL) on_rx();
        if (g & G_IEPINT) on_in();
        if (g & G_OEPINT) on_out_events();
    }
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
