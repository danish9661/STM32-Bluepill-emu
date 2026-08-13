/*
 * arduino_ws2812.ino — 8-LED WS2812 strip driven over SPI1 + DMA1 (no library).
 *
 * WS2812 data protocol: 800 kHz bit rate, 1-bit = 3 SPI clock cycles at
 * 2.25 MHz (72 MHz / 32): "1" -> 0b110, "0" -> 0b100. 24 bits per LED
 * (GRB order), 9 SPI bytes per LED, 72 bytes per 8-LED frame. Frames are
 * separated by a >50 us reset (data line low) — here an explicit delay.
 *
 * The page decodes the SPI1 DR write stream (onPeriphWrite) into the strip
 * rendering — no CS pin, MOSI (PA7) is the data line.
 */

#define N_LEDS 8
#define SPI_BYTES_PER_FRAME (N_LEDS * 9) /* 8 LEDs * 24 ws bits * 3 spi bits / 8 */

#include <SPI.h>

static uint8_t enc[256][3];     /* data byte -> 3 SPI bytes (24 bits, MSB first) */
static uint8_t frame[N_LEDS * 3]; /* GRB per LED */
static uint8_t spi_buf[SPI_BYTES_PER_FRAME];
static volatile uint32_t ws_frames = 0;

static void build_encoder(void) {
    for (int d = 0; d < 256; d++) {
        uint32_t acc = 0;
        for (int b = 7; b >= 0; b--) {
            acc = (acc << 3) | (((d >> b) & 1) ? 0b110 : 0b100);
        }
        enc[d][0] = (uint8_t)(acc >> 16);
        enc[d][1] = (uint8_t)(acc >> 8);
        enc[d][2] = (uint8_t)acc;
    }
}

/* hue (0-359) -> RGB 0-255 */
static void hsv2rgb(uint16_t h, uint8_t *r, uint8_t *g, uint8_t *b) {
    uint8_t region = (uint8_t)(h / 60);
    uint8_t rem = (uint8_t)(h % 60);
    uint8_t q = (uint8_t)(255 * (60 - rem) / 60);
    uint8_t t = (uint8_t)(255 * rem / 60);
    switch (region) {
        case 0: *r = 255; *g = t; *b = 0; break;
        case 1: *r = q; *g = 255; *b = 0; break;
        case 2: *r = 0; *g = 255; *b = t; break;
        case 3: *r = 0; *g = q; *b = 255; break;
        case 4: *r = t; *g = 0; *b = 255; break;
        default: *r = 255; *g = 0; *b = q; break;
    }
}

static void render_rainbow(uint32_t frame_idx) {
    for (int i = 0; i < N_LEDS; i++) {
        uint8_t r, g, b;
        hsv2rgb((uint16_t)((frame_idx * 9 + i * 45) % 360), &r, &g, &b);
        frame[i * 3 + 0] = g; /* GRB: green first */
        frame[i * 3 + 1] = r;
        frame[i * 3 + 2] = b;
    }
}

static void send_frame_dma(void) {
    for (int i = 0; i < N_LEDS * 3; i++) {
        spi_buf[i * 3 + 0] = enc[frame[i]][0];
        spi_buf[i * 3 + 1] = enc[frame[i]][1];
        spi_buf[i * 3 + 2] = enc[frame[i]][2];
    }
    /* clear any stale TC flag, then fire-and-forget: mem -> SPI1 DR, 8-bit, no IRQ */
    DMA1->IFCR = DMA_IFCR_CTCIF3;
    DMA1_Channel3->CCR = 0;
    DMA1_Channel3->CPAR = (uint32_t)&SPI1->DR;
    DMA1_Channel3->CMAR = (uint32_t)spi_buf;
    DMA1_Channel3->CNDTR = SPI_BYTES_PER_FRAME;
    DMA1_Channel3->CCR = DMA_CCR_MINC | DMA_CCR_DIR | DMA_CCR_EN; /* 8-bit: PSIZE/MSIZE=00 — an 8-bit SPI (DS=8) clocks 8 bits per 16-bit DR write */
    SPI1->CR2 |= SPI_CR2_TXDMAEN;
    while ((DMA1->ISR & DMA_ISR_TCIF3) == 0) { /* pump completes it across batches */
    }
    SPI1->CR2 &= ~SPI_CR2_TXDMAEN;
    DMA1->IFCR = DMA_IFCR_CTCIF3;
    DMA1_Channel3->CCR = 0;
    ws_frames++;
}

void setup() {
    Serial1.begin(115200);
    build_encoder();
    pinMode(PC13, OUTPUT);
    digitalWrite(PC13, HIGH);

    SPI.begin();
    SPI.setClockDivider(SPI_CLOCK_DIV32); /* 72 MHz / 32 = 2.25 MHz -> 800 kHz ws bits */

    Serial1.print("\r\n=== WS2812 strip over SPI1 + DMA1 ch3 ===\r\n");
    Serial1.print("SPI=2.25MHz leds=8 frame_bytes=72 dma=DMA1_Channel3\r\n");
    Serial1.print("WS2812=ok\r\n");
}

void loop() {
    static uint32_t last_print = 0;
    render_rainbow(ws_frames);
    send_frame_dma();
    delay(30); /* frame pacing + >50us reset gap between frames */

    if (millis() - last_print >= 2000) {
        last_print = millis();
        Serial1.print("WS2812 frames=");
        Serial1.print(ws_frames);
        Serial1.print("\r\n");
    }
}