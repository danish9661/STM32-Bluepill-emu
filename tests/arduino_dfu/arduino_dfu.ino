// STM32 Maple-style USB DFU bootloader demo (register-level USB FS device).
//
// Answers host enumeration (device / configuration descriptors with the DFU
// functional descriptor, SET_ADDRESS, SET_CONFIGURATION), then serves the
// USB DFU class (DNLOAD/UPLOAD/GETSTATUS/GETSTATE/CLRSTATUS/ABORT) like the
// Maple Mini factory bootloader: DNLOAD blocks program flash at the DFU
// download address (default 0x08005000, settable via SetAddressPointer),
// the manifest request completes the image. Progress prints on Serial
// (USART1); an exact-path RAM trace records every stage for tests.
//
// Notes on fidelity (all documented, all verified in tests/test_dfu.mjs):
// - wTransferSize is 64 (Maple uses 1024); blocks: 0 = commands/manifest,
//   >= 1 = 64 B data at dfu_addr + (block-1)*64.
// - Downloaded bytes additionally land in a RAM stage buffer (dfu_stage):
//   guest stores to flash are dropped by the memory model, so the unlock /
//   program / BSY sequence runs for realism but verification reads the
//   stage image (resolved from ELF symbols, like canRxArmed).
// - Manifest records completion in RAM; no reboot into the image happens
//   (jumping to unverified guest code is unpredictable).
// - DNBUSY timing is collapsed: programming completes synchronously, so
//   GETSTATUS moves SYNC -> IDLE directly (a real host polls through it).
// Drive it headless with usb_inject_setup / usb_inject_out and read the
// replies from UsbIn events (see tests/test_dfu.mjs).
// malloc-free (no String/new).
#include <Arduino.h>

#define USB_B   0x40005C00u
#define PMA_B   0x40006000u
#define FLASH_B 0x40022000u
#define reg(a) (*(volatile uint32_t *)(a))
#define preg(a) (*(volatile uint16_t *)(a))
#define pbreg(a) (*(volatile uint8_t *)(a))

// Endpoint register offsets + bits.
#define EP0R 0x00
#define CNTR 0x40
#define ISTR 0x44
#define DADDR 0x4C
#define BTABLE 0x50
#define CTR_RX 0x8000u
#define CTR_TX 0x0080u
#define STAT_TX_M 0x0030u
#define STAT_RX_M 0x3000u
#define ISTR_CTR (1u << 15)
#define ISTR_DIR (1u << 4)

// FLASH programming interface.
#define FLASH_KEYR 0x04
#define FLASH_SR   0x08
#define FLASH_CR   0x0C
#define FLASH_SR_BSY 1u
#define FLASH_CR_PG  1u
#define FLASH_CR_LOCK (1u << 7)

// PMA layout (same as the CDC demo: BTABLE = 0, 16 B stride/endpoint).
#define ADDR0_TX 0x00
#define COUNT0_TX 0x04
#define ADDR0_RX 0x08
#define COUNT0_RX 0x0C
#define BUF0_TX 0x20
#define BUF0_RX 0x30

// EP0R direct-field keep values (EA | TYPE): STAT bits written 0
// never toggle, CTR bits written 0 clear.
#define EP0_KEEP 0x0200u  // EA=0, TYPE=control

// Set STAT_TX/RX to target states from ANY current state (toggle math).
static void ep_stat(uint32_t epr, uint32_t keep, uint32_t tx, uint32_t rx) {
    uint32_t cur = reg(USB_B + epr);
    uint32_t tm = (((cur >> 4) & 3) ^ (tx & 3)) << 4;
    uint32_t rm = (((cur >> 12) & 3) ^ (rx & 3)) << 12;
    reg(USB_B + epr) = keep | tm | rm;
}
// EP0R write values must set the OPPOSITE direction's CTR bit to 1
// (write-1-no-effect preserves a pending completion flag) — see the CDC
// demo for why a bare EP0_KEEP write wedges multi-packet IN transfers.
static void ep0_tx_valid(void) {  // TX -> VALID, RX untouched, RX CTR kept
    uint32_t cur = reg(USB_B + EP0R);
    reg(USB_B + EP0R) = EP0_KEEP | CTR_RX | ((((cur >> 4) & 3) ^ 3) << 4);
}
static void ep0_rx_valid(void) {  // RX -> VALID, TX untouched, TX CTR kept
    uint32_t cur = reg(USB_B + EP0R);
    reg(USB_B + EP0R) = EP0_KEEP | CTR_TX | ((((cur >> 12) & 3) ^ 3) << 12);
}
static inline void pma_write(uint32_t word, const uint8_t *src, uint32_t len) {
    for (uint32_t i = 0; i < len; i++)
        pbreg(PMA_B + word * 2 + (i >> 1) * 4 + (i & 1)) = src[i];
}
static inline void pma_read(uint32_t word, uint8_t *dst, uint32_t len) {
    for (uint32_t i = 0; i < len; i++)
        dst[i] = pbreg(PMA_B + word * 2 + (i >> 1) * 4 + (i & 1));
}

// --- Descriptors ---------------------------------------------------------
static const uint8_t DEV_DESC[] = {
    18, 1, 0x00, 0x02, 0x00, 0x00, 0x00, 64,
    0xAF, 0x1E, 0x03, 0x00, 0x00, 0x01, 1, 2, 0, 1,
};
static const uint8_t CFG_DESC[] = {
    // config (9): wTotalLength 27, 1 interface
    9, 2, 27, 0, 1, 1, 0, 0x80, 100,
    // interface (9): Application / DFU / DFU-mode, 0 endpoints
    9, 4, 0, 0, 0, 0xFE, 0x01, 0x02, 0,
    // DFU functional (9): bcdDFU 0x011A, canDnload|canUpload|manifestTolerant
    9, 0x21, 0x0B, 0x00, 0x00, 64, 0, 0x1A, 0x01,
};
static const uint8_t STR0_DESC[] = { 4, 3, 0x09, 0x04 };
static const uint8_t STR1_DESC[] = {
    8, 3, 'E', 0, 'm', 0, 'u', 0,
};
static const uint8_t STR2_DESC[] = {
    18, 3, 'D', 0, 'F', 0, 'U', 0, ' ', 0, 'd', 0, 'e', 0, 'm', 0, 'o', 0,
};

// --- DFU state ------------------------------------------------------------
// DFU 1.1 states (only the download path is exercised).
#define DFU_IDLE 2
#define DFU_DNLOAD_SYNC 3
#define DFU_DNBUSY 4
#define DFU_DNLOAD_IDLE 5
#define DFU_MANIFEST_SYNC 6
#define DFU_MANIFEST 7
#define DFU_UPLOAD_IDLE 9
#define DFU_ERROR 10
// DFU class requests.
#define DFU_DETACH 0
#define DFU_DNLOAD 1
#define DFU_UPLOAD 2
#define DFU_GETSTATUS 3
#define DFU_CLRSTATUS 4
#define DFU_GETSTATE 5
#define DFU_ABORT 6
// Special DNLOAD command prefix (block 0): SetAddressPointer.
#define DFU_CMD_SETADDR 0x21
// Download window: Maple sketch offset to top of 64K flash.
#define DFU_BASE 0x08005000u
#define DFU_END  0x08010000u
#define DFU_XFER 64u
#define DFU_STAGE_LEN 2048u

static uint8_t dfu_state = DFU_IDLE;
static uint32_t dfu_addr = DFU_BASE;      // current download pointer
static uint8_t dfu_stage[DFU_STAGE_LEN];  // staged image (flash stores drop)
static uint32_t dfu_staged = 0;           // staged bytes total
static uint16_t dnload_block = 0;         // block of the in-flight OUT stage
static uint16_t dnload_len = 0;           // its byte length (clamped to 64)

// Exact-path trace for tests (resolved from ELF symbols).
static volatile uint32_t dfu_trace[8];
static volatile uint32_t dfu_trace_n = 0;
static void trace_push(uint32_t c) {
    if (dfu_trace_n < 8) dfu_trace[dfu_trace_n++] = c;
}

// --- EP0 control state ----------------------------------------------------
enum Ep0State { IDLE, IN_DATA, IN_STATUS, OUT_DATA };
static Ep0State ep0state = IDLE;
static const uint8_t *in_ptr = 0;
static uint32_t in_left = 0;
static uint8_t pending_addr = 0;
static bool addr_pending = false;

static void ep0_tx(const uint8_t *src, uint32_t len) {
    pma_write(BUF0_TX, src, len);
    preg(PMA_B + COUNT0_TX) = (uint16_t)len;
    ep0_tx_valid();
}
static void ep0_tx_status(void) {
    preg(PMA_B + COUNT0_TX) = 0;
    ep0_tx_valid();
}
static void ep0_clear_rx(void) {
    reg(USB_B + EP0R) = EP0_KEEP | CTR_TX;  // retire RX CTR, keep a TX flag
}
static void ep0_clear_tx(void) {
    reg(USB_B + EP0R) = EP0_KEEP | CTR_RX;  // retire TX CTR, keep an RX flag
}

// --- Flash programming (real unlock/program sequence) --------------------
static void flash_unlock(void) {
    reg(FLASH_B + FLASH_KEYR) = 0x45670123;
    reg(FLASH_B + FLASH_KEYR) = 0xCDEF89AB;
}
static void flash_program(uint32_t addr, const uint8_t *src, uint32_t len) {
    flash_unlock();
    reg(FLASH_B + FLASH_CR) |= FLASH_CR_PG;
    for (uint32_t i = 0; i < len; i += 2) {
        uint16_t half = src[i] | ((uint16_t)(i + 1 < len ? src[i + 1] : 0xFF) << 8);
        *(volatile uint16_t *)(addr + i) = half;
        // NOTE: no BSY poll here — the model asserts BSY while PG is set
        // but guest flash stores bypass the peripheral, so no completion
        // ever clears it (polling would spin forever). PG set/clear around
        // the loop is the observable unlock/program sequence.
    }
    reg(FLASH_B + FLASH_CR) &= ~FLASH_CR_PG;
}

static uint8_t dfu_status[6];
static void ep0_dfu_status(void) {
    dfu_status[0] = 0;  // bStatus OK
    dfu_status[1] = dfu_status[2] = dfu_status[3] = 0;  // bwPollTimeout
    dfu_status[4] = dfu_state;
    dfu_status[5] = 0;  // iString
    ep0_tx(dfu_status, 6);
}

static void dfu_error(uint32_t code) {
    dfu_state = DFU_ERROR;
    trace_push(0xE0 | code);
    ep_stat(EP0R, EP0_KEEP, 1, 1);  // stall both directions
    ep0state = IDLE;
}

static void handle_setup(void) {
    uint8_t s[8];
    pma_read(BUF0_RX, s, 8);
    ep0_clear_rx();  // retire RX CTR (+SETUP) first: ISTR must not re-fire
    uint8_t req_type = s[0], req = s[1];
    uint16_t wValue = s[2] | ((uint16_t)s[3] << 8);
    uint16_t wIndex = s[4] | ((uint16_t)s[5] << 8);
    uint16_t wLength = s[6] | ((uint16_t)s[7] << 8);
    (void)wIndex;
    if (req_type == 0x80 && req == 0x06) {          // GET_DESCRIPTOR
        const uint8_t *d = 0;
        uint32_t n = 0;
        uint8_t dtype = wValue >> 8, didx = wValue & 0xFF;
        if (dtype == 1 && didx == 0) { d = DEV_DESC; n = sizeof(DEV_DESC); }
        else if (dtype == 2 && didx == 0) { d = CFG_DESC; n = sizeof(CFG_DESC); }
        else if (dtype == 3 && didx == 0) { d = STR0_DESC; n = sizeof(STR0_DESC); }
        else if (dtype == 3 && didx == 1) { d = STR1_DESC; n = sizeof(STR1_DESC); }
        else if (dtype == 3 && didx == 2) { d = STR2_DESC; n = sizeof(STR2_DESC); }
        if (d && n) {
            if (n > wLength) n = wLength;
            in_ptr = d; in_left = n;
            uint32_t chunk = n > 64 ? 64 : n;
            ep0_tx(in_ptr, chunk);
            in_ptr += chunk; in_left -= chunk;
            ep0state = in_left ? IN_DATA : IN_STATUS;
            return;
        }
    } else if (req_type == 0x00 && req == 0x05) {   // SET_ADDRESS
        pending_addr = (uint8_t)wValue;
        addr_pending = true;
        ep0_tx_status();
        ep0state = IN_STATUS;
        return;
    } else if (req_type == 0x00 && req == 0x09) {   // SET_CONFIGURATION
        dfu_state = DFU_IDLE;
        trace_push(1);
        Serial.println("dfu: configured");
        ep0_tx_status();
        ep0state = IN_STATUS;
        return;
    } else if (req_type == 0x21 && req == DFU_DETACH) {
        ep0_tx_status();
        ep0state = IN_STATUS;
        return;
    } else if (req_type == 0x21 && req == DFU_DNLOAD) {
        uint16_t block = wValue;
        if (wLength == 0) {
            // Zero-length DNLOAD on block 0 = manifest: image complete.
            if (block == 0) {
                dfu_state = DFU_MANIFEST;
                trace_push(4);
                Serial.println("dfu: manifest complete");
                ep0_tx_status();
                ep0state = IN_STATUS;
                return;
            }
            dfu_error(1);
            return;
        }
        if (block == 0) {
            // Command stage (OUT data follows); handled on completion.
            dnload_block = 0;
            dnload_len = wLength > 64 ? 64 : wLength;
            ep0state = OUT_DATA;
            ep0_rx_valid();
            return;
        }
        dnload_block = block;
        dnload_len = wLength > 64 ? 64 : wLength;
        dfu_state = DFU_DNLOAD_SYNC;
        ep0state = OUT_DATA;
        ep0_rx_valid();  // arm the OUT data stage
        return;
    } else if (req_type == 0xA1 && req == DFU_UPLOAD) {
        uint16_t block = wValue;
        if (block == 0 || wLength == 0) { dfu_error(2); return; }
        // Serve staged bytes (short packet at the image end is fine).
        uint32_t off = (uint32_t)(block - 1) * DFU_XFER;
        uint32_t n = wLength;
        static uint8_t up[64];
        for (uint32_t i = 0; i < n && i < 64; i++)
            up[i] = (off + i < dfu_staged) ? dfu_stage[off + i] : 0xFF;
        if (n > 64) n = 64;
        dfu_state = DFU_UPLOAD_IDLE;
        in_ptr = up; in_left = n;
        {
            uint32_t chunk = n > 64 ? 64 : n;
            ep0_tx(in_ptr, chunk);
            in_ptr += chunk; in_left -= chunk;
        }
        ep0state = in_left ? IN_DATA : IN_STATUS;
        return;
    } else if (req_type == 0xA1 && req == DFU_GETSTATUS) {
        ep0_dfu_status();
        ep0state = IN_STATUS;
        return;
    } else if (req_type == 0x21 && req == DFU_CLRSTATUS) {
        dfu_state = DFU_IDLE;
        ep0_tx_status();
        ep0state = IN_STATUS;
        return;
    } else if (req_type == 0xA1 && req == DFU_GETSTATE) {
        ep0_tx(&dfu_state, 1);
        ep0state = IN_STATUS;
        return;
    } else if (req_type == 0x21 && req == DFU_ABORT) {
        dfu_state = DFU_IDLE;
        trace_push(5);
        ep0_tx_status();
        ep0state = IN_STATUS;
        return;
    }
    // Unknown: stall both directions (not exercised by the test).
    ep_stat(EP0R, EP0_KEEP, 1, 1);
    ep0state = IDLE;
}

static void handle_out_data(void) {
    // DNLOAD OUT-data stage completion: block 0 = commands, >= 1 = program.
    static uint8_t buf[64];
    uint32_t n = dnload_len;
    if (n > 64) n = 64;
    pma_read(BUF0_RX, buf, n);
    ep0_clear_rx();
    if (dnload_block == 0) {
        // Command stage: only SetAddressPointer (0x21 + LE32 address).
        if (n >= 5 && buf[0] == DFU_CMD_SETADDR) {
            uint32_t addr = (uint32_t)buf[1] | ((uint32_t)buf[2] << 8) |
                            ((uint32_t)buf[3] << 16) | ((uint32_t)buf[4] << 24);
            if (addr < DFU_BASE || addr >= DFU_END) { dfu_error(3); return; }
            dfu_addr = addr;
            dfu_state = DFU_DNLOAD_IDLE;
            trace_push(2);
            Serial.println("dfu: address set");
        } else {
            dfu_error(4);
            return;
        }
    } else {
        uint32_t off = dfu_addr - DFU_BASE + (uint32_t)(dnload_block - 1) * DFU_XFER;
        uint32_t addr = DFU_BASE + (off % (DFU_END - DFU_BASE));
        // Stage in RAM (guest flash stores drop) + run the real program seq.
        for (uint32_t i = 0; i < n && dfu_staged < DFU_STAGE_LEN; i++)
            dfu_stage[dfu_staged++] = buf[i];
        flash_program(addr, buf, n);
        dfu_state = DFU_DNLOAD_IDLE;
        trace_push(3);
        Serial.println("dfu: block programmed");
    }
    ep0_tx_status();
    ep0state = IN_STATUS;
}

static void poll_usb(void) {
    uint32_t istr = reg(USB_B + ISTR);
    if (istr & (1u << 10)) {                        // RESET
        reg(USB_B + ISTR) = istr & ~(1u << 10);
        reg(USB_B + EP0R) = 0x3200;                 // EP0 control, RX VALID
        ep0state = IDLE;
        addr_pending = false;
        dfu_state = DFU_IDLE;
        Serial.println("dfu: reset");
        return;
    }
    if (!(istr & ISTR_CTR)) return;
    uint32_t ep_id = istr & 0xF;
    bool dir = (istr & ISTR_DIR) != 0;
    if (ep_id == 0 && dir) {                        // EP0 RX (SETUP or OUT)
        if (reg(USB_B + EP0R) & 0x0800) {           // SETUP bit
            handle_setup();
        } else if (ep0state == OUT_DATA) {          // DNLOAD OUT-data done
            handle_out_data();
        } else if (ep0state == IN_STATUS) {         // status-OUT completion
            ep0_clear_rx();
            ep0state = IDLE;
        } else {
            ep0_clear_rx();
        }
        // Control endpoint rests armed (same ordering-race close as CDC).
        ep0_rx_valid();
    } else if (ep_id == 0 && !dir) {                // EP0 TX done
        ep0_clear_tx();
        if (ep0state == IN_DATA) {
            if (in_left) {
                uint32_t chunk = in_left > 64 ? 64 : in_left;
                ep0_tx(in_ptr, chunk);
                in_ptr += chunk; in_left -= chunk;
            } else {
                ep0state = IN_STATUS;
                ep0_rx_valid();  // arm the status-OUT stage now that IN is done
            }
        } else if (ep0state == IN_STATUS) {
            if (addr_pending) {
                reg(USB_B + DADDR) = 0x80 | pending_addr;
                addr_pending = false;
                Serial.print("dfu: addr=");
                Serial.println(pending_addr);
            }
            ep0state = IDLE;
            ep0_rx_valid();
        }
    }
}

void setup() {
    pinMode(PC13, OUTPUT);
    Serial.begin(115200);
    Serial.println("\r\n=== USB DFU bootloader demo (Maple-style DNLOAD + manifest) ===");
    RCC->APB1ENR |= (1 << 23);   // USBEN
    reg(USB_B + BTABLE) = 0;
    // PMA buffer table: TX/RX addr + RX block config (64B: BL 32B x2).
    preg(PMA_B + ADDR0_TX) = BUF0_TX;
    preg(PMA_B + COUNT0_TX) = 0;
    preg(PMA_B + ADDR0_RX) = BUF0_RX;
    preg(PMA_B + COUNT0_RX) = 0x8800;
    // Attach: clear FRES+PDWN with CTRM+RESETM armed -> RESET event.
    reg(USB_B + CNTR) = (1u << 15) | (1u << 10);
}

void loop() {
    poll_usb();
}
