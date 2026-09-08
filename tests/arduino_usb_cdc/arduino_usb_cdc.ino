// STM32 Bluepill WASM emulator demo: USB CDC-ACM virtual serial port.
//
// Register-level USB device (no HAL): answers host enumeration (device /
// configuration / string descriptors, SET_ADDRESS, SET_CONFIGURATION, CDC
// SET_LINE_CODING + SET_CONTROL_LINE_STATE) then echoes bulk traffic on
// EP1 (OUT -> IN loopback). Progress prints on Serial (USART1).
// Drive it headless with usb_inject_setup / usb_inject_out and read the
// replies from UsbIn events (see tests/test_usb_cdc.mjs).
// malloc-free (no String/new).
#include <Arduino.h>

#define USB_B   0x40005C00u
#define PMA_B   0x40006000u
#define reg(a) (*(volatile uint32_t *)(a))
#define preg(a) (*(volatile uint16_t *)(a))

// Endpoint register offsets + bits.
#define EP0R 0x00
#define EP1R 0x04
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

// PMA layout (BTABLE = 0).
#define ADDR0_TX 0x00
#define COUNT0_TX 0x02
#define ADDR0_RX 0x04
#define COUNT0_RX 0x06
#define ADDR1_TX 0x08
#define COUNT1_TX 0x0A
#define ADDR1_RX 0x0C
#define COUNT1_RX 0x0E
#define BUF0_TX 0x40
#define BUF0_RX 0x80
#define BUF1_TX 0xC0
#define BUF1_RX 0x100

// EP0R/EP1R direct-field keep values (EA | TYPE): STAT bits written 0
// never toggle, CTR bits written 0 clear.
#define EP0_KEEP 0x0200u  // EA=0, TYPE=control
#define EP1_KEEP 0x0001u  // EA=1, TYPE=bulk

// Set STAT_TX/RX to target states from ANY current state (toggle math:
// per-field mask = current ^ target). Direct fields rewritten to keep
// values; CTR bits written 0 clear.
static void ep_stat(uint32_t epr, uint32_t keep, uint32_t tx, uint32_t rx) {
    uint32_t cur = reg(USB_B + epr);
    uint32_t tm = (((cur >> 4) & 3) ^ (tx & 3)) << 4;
    uint32_t rm = (((cur >> 12) & 3) ^ (rx & 3)) << 12;
    reg(USB_B + epr) = keep | tm | rm;
}
// EP0R/EP1R write values must set the OPPOSITE direction's CTR bit to 1
// (write-1-no-effect preserves a pending completion flag) and 0 to the own
// direction's CTR only when retiring it. A bare EP0_KEEP write clears a
// just-raised TX completion before the TX branch sees it — this wedged
// multi-packet IN transfers (config descriptor 2nd packet never staged).
static void ep0_tx_valid(void) {  // TX -> VALID, RX untouched, RX CTR kept
    uint32_t cur = reg(USB_B + EP0R);
    reg(USB_B + EP0R) = EP0_KEEP | CTR_RX | ((((cur >> 4) & 3) ^ 3) << 4);
}
static void ep0_rx_valid(void) {  // RX -> VALID, TX untouched, TX CTR kept
    uint32_t cur = reg(USB_B + EP0R);
    reg(USB_B + EP0R) = EP0_KEEP | CTR_TX | ((((cur >> 12) & 3) ^ 3) << 12);
}
static void ep1_tx_valid(void) {  // TX -> VALID, RX untouched, RX CTR kept
    uint32_t cur = reg(USB_B + EP1R);
    reg(USB_B + EP1R) = EP1_KEEP | CTR_RX | ((((cur >> 4) & 3) ^ 3) << 4);
}
static void ep1_rx_valid(void) {  // RX -> VALID, TX untouched, TX CTR kept
    uint32_t cur = reg(USB_B + EP1R);
    reg(USB_B + EP1R) = EP1_KEEP | CTR_TX | ((((cur >> 12) & 3) ^ 3) << 12);
}
static inline void pma_write(uint32_t off, const uint8_t *src, uint32_t len) {
    for (uint32_t i = 0; i < len; i++)
        preg(PMA_B + off + i) = src[i];
}
static inline void pma_read(uint32_t off, uint8_t *dst, uint32_t len) {
    for (uint32_t i = 0; i < len; i++)
        dst[i] = (uint8_t)preg(PMA_B + off + i);
}

// --- Descriptors ---------------------------------------------------------
static const uint8_t DEV_DESC[] = {
    18, 1, 0x00, 0x02, 0xEF, 0x02, 0x01, 64,
    0x83, 0x04, 0x40, 0x57, 0x00, 0x01, 1, 2, 0, 1,
};
static const uint8_t CFG_DESC[] = {
    // config (9)
    9, 2, 68, 0, 2, 1, 0, 0x80, 100,
    // IAD (8)
    8, 11, 0, 2, 0x02, 0x02, 0x01, 0,
    // control interface (9)
    9, 4, 0, 0, 1, 0x02, 0x02, 0x01, 0,
    // CDC header (5)
    5, 0x24, 0x00, 0x10, 0x01,
    // call management (5)
    5, 0x24, 0x01, 0x00, 1,
    // ACM (4)
    4, 0x24, 0x02, 0x02,
    // union (5)
    5, 0x24, 0x06, 0, 1,
    // data interface (9)
    9, 4, 1, 0, 2, 0x0A, 0x00, 0x00, 0,
    // EP1 OUT bulk 64B (7)
    7, 5, 0x01, 0x02, 64, 0, 0,
    // EP1 IN bulk 64B (7)
    7, 5, 0x81, 0x02, 64, 0, 0,
};
static const uint8_t STR0_DESC[] = { 4, 3, 0x09, 0x04 };
static const uint8_t STR1_DESC[] = {
    10, 3, 'M', 0, 'U', 0, 'S', 0, 'E', 0,
};
static const uint8_t STR2_DESC[] = {
    22, 3, 'B', 0, 'l', 0, 'u', 0, 'e', 0, 'p', 0, 'i', 0, 'l', 0, 'l', 0, 'C', 0, 'D', 0, 'C', 0,
};

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

static void handle_setup(void) {
    uint8_t s[8];
    pma_read(BUF0_RX, s, 8);
    ep0_clear_rx();  // retire RX CTR (+SETUP) first: ISTR must not re-fire
    uint8_t req_type = s[0], req = s[1];
    uint16_t wValue = s[2] | ((uint16_t)s[3] << 8);
    uint16_t wLength = s[6] | ((uint16_t)s[7] << 8);
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
            // NOTE: RX stays NAK'd until the IN transfer completes; the
            // status-OUT stage is armed when the last packet completes
            // (arming here would clear the TX completion flag via the
            // CTR-0 bits in the EP0R write).
            return;
        }
    } else if (req_type == 0x00 && req == 0x05) {   // SET_ADDRESS
        pending_addr = (uint8_t)wValue;
        addr_pending = true;
        ep0_tx_status();
        ep0state = IN_STATUS;
        return;
    } else if (req_type == 0x00 && req == 0x09) {   // SET_CONFIGURATION
        ep0_tx_status();
        ep0state = IN_STATUS;
        Serial.println("usb: configured");
        return;
    } else if (req_type == 0x21 && req == 0x20) {   // SET_LINE_CODING
        ep0state = OUT_DATA;
        ep0_rx_valid();  // arm 7-byte OUT data stage
        return;
    } else if (req_type == 0x21 && req == 0x22) {   // SET_CONTROL_LINE_STATE
        ep0_tx_status();
        ep0state = IN_STATUS;
        return;
    }
    // Unknown: stall both directions (not exercised by the test).
    ep_stat(EP0R, EP0_KEEP, 1, 1);
    ep0state = IDLE;
}

static void poll_usb(void) {
    uint32_t istr = reg(USB_B + ISTR);
    if (istr & (1u << 10)) {                        // RESET
        reg(USB_B + ISTR) = istr & ~(1u << 10);
        reg(USB_B + EP0R) = 0x3200;                 // EP0 control, RX VALID
        reg(USB_B + EP1R) = 0x3001;                 // EP1 bulk, RX VALID
        ep0state = IDLE;
        addr_pending = false;
        Serial.println("usb: reset");
        return;
    }
    if (!(istr & ISTR_CTR)) return;
    uint32_t ep_id = istr & 0xF;
    bool dir = (istr & ISTR_DIR) != 0;
    if (ep_id == 0 && dir) {                        // EP0 RX (SETUP or OUT)
        if (reg(USB_B + EP0R) & 0x0800) {           // SETUP bit
            handle_setup();
        } else if (ep0state == OUT_DATA) {          // SET_LINE_CODING data
            ep0_clear_rx();
            ep0_tx_status();
            ep0state = IN_STATUS;
            Serial.println("usb: line coding set");
        } else if (ep0state == IN_STATUS) {         // status-OUT completion
            ep0_clear_rx();
            ep0state = IDLE;
        } else {
            ep0_clear_rx();
        }
        // Control endpoint rests armed: any RX service ends with RX VALID
        // (idempotent toggle — already-VALID is a no-op). This closes the
        // TX-complete/status-OUT ordering race in both directions: the
        // next SETUP is accepted no matter which completion landed first.
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
                Serial.print("usb: addr=");
                Serial.println(pending_addr);
            }
            ep0state = IDLE;
            ep0_rx_valid();  // ready for the next SETUP (idempotent: also
            // armed by the RX branch; whichever runs last wins harmlessly)
        }
    } else if (ep_id == 1 && dir) {                 // EP1 OUT: echo it back
        uint32_t n = preg(PMA_B + COUNT1_RX) & 0x3FF;
        if (n > 64) n = 64;
        static uint8_t echo[64];
        pma_read(BUF1_RX, echo, n);
        reg(USB_B + EP1R) = EP1_KEEP | CTR_TX;      // clear RX CTR, keep TX
        ep1_rx_valid();                             // re-arm RX VALID
        pma_write(BUF1_TX, echo, n);
        preg(PMA_B + COUNT1_TX) = (uint16_t)n;
        ep1_tx_valid();                             // TX VALID -> UsbIn
        Serial.print("usb: echo ");
        Serial.println(n);
    } else if (ep_id == 1 && !dir) {                // EP1 TX done
        reg(USB_B + EP1R) = EP1_KEEP | CTR_RX;      // clear TX CTR, keep RX
    }
}

void setup() {
    pinMode(PC13, OUTPUT);
    Serial.begin(115200);
    Serial.println("\r\n=== USB CDC serial demo (EP0 control + EP1 bulk echo) ===");
    RCC->APB1ENR |= (1 << 23);   // USBEN
    reg(USB_B + BTABLE) = 0;
    // PMA buffer table: TX/RX addr + RX block config (64B: BL 32B x2).
    preg(PMA_B + ADDR0_TX) = BUF0_TX;
    preg(PMA_B + COUNT0_TX) = 0;
    preg(PMA_B + ADDR0_RX) = BUF0_RX;
    preg(PMA_B + COUNT0_RX) = 0x8800;
    preg(PMA_B + ADDR1_TX) = BUF1_TX;
    preg(PMA_B + COUNT1_TX) = 0;
    preg(PMA_B + ADDR1_RX) = BUF1_RX;
    preg(PMA_B + COUNT1_RX) = 0x8800;
    // Attach: clear FRES+PDWN with CTRM+RESETM armed -> RESET event.
    reg(USB_B + CNTR) = (1u << 15) | (1u << 10);
}

void loop() {
    poll_usb();
}
