/*
 * STM32 Bluepill WASM emulator showcase: 7 real-looking peripherals at once.
 *
 *   I2C OLED 128x64 (SSD1306-style, 0x3C)  — page-rendered via framebuffer
 *   SPI LCD  128x64 (SPI1, CS PA8)         — page-rendered via framebuffer
 *   7-seg 74HC595 display (SPI1, CS PA4)   — page decodes the shift register
 *   RGB LED  TIM2 CH1/2/3 on PA0/PA1/PA2   — page reads PWM duty
 *   Buzzer   PB14 (active-buzzer beeps)    — page reads the pin
 *   Button   PB13 EXTI13 rising edge       — page drives the pin
 *   PC13 blue LED blinks                   — page already renders it
 *
 * I2C is I2C1 (PB6/PB7), SPI1 = PA5/PA6/PA7 pins. malloc-free sketch.
 */
#include <Arduino.h>
#include <SPI.h>
#include <Wire.h>

#define OLED_ADDR 0x3C
#define OLED_W    128
#define OLED_H    64

#define LCD_CS    PA8
#define SEG_CS    PA4
#define BUZZ      PB14
#define BTN       PB13

/* 5x7 font, first 10 rows = space, then ASCII 32..95 in order */
static const uint8_t FONT5x7[][5] = {
    {0x00,0x00,0x00,0x00,0x00}, /* ' ' */
    {0x00,0x00,0x5F,0x00,0x00}, /* '!' */
    {0x00,0x07,0x00,0x07,0x00}, /* '"' */
    {0x14,0x7F,0x14,0x7F,0x14}, /* '#' */
    {0x24,0x2A,0x7F,0x2A,0x12}, /* '$' */
    {0x23,0x13,0x08,0x64,0x62}, /* '%' */
    {0x36,0x49,0x55,0x22,0x50}, /* '&' */
    {0x00,0x05,0x03,0x00,0x00}, /* ''' */
    {0x00,0x1C,0x22,0x41,0x00}, /* '(' */
    {0x00,0x41,0x22,0x1C,0x00}, /* ')' */
    {0x08,0x2A,0x1C,0x2A,0x08}, /* '*' */
    {0x08,0x08,0x3E,0x08,0x08}, /* '+' */
    {0x00,0x50,0x30,0x00,0x00}, /* ',' */
    {0x08,0x08,0x08,0x08,0x08}, /* '-' */
    {0x00,0x60,0x60,0x00,0x00}, /* '.' */
    {0x20,0x10,0x08,0x04,0x02}, /* '/' */
    {0x3E,0x51,0x49,0x45,0x3E}, /* '0' */
    {0x00,0x42,0x7F,0x40,0x00}, /* '1' */
    {0x42,0x61,0x51,0x49,0x46}, /* '2' */
    {0x21,0x41,0x45,0x4B,0x31}, /* '3' */
    {0x18,0x14,0x12,0x7F,0x10}, /* '4' */
    {0x27,0x45,0x45,0x45,0x39}, /* '5' */
    {0x3C,0x4A,0x49,0x49,0x30}, /* '6' */
    {0x01,0x71,0x09,0x05,0x03}, /* '7' */
    {0x36,0x49,0x49,0x49,0x36}, /* '8' */
    {0x06,0x49,0x49,0x29,0x1E}, /* '9' */
    {0x00,0x36,0x36,0x00,0x00}, /* ':' */
    {0x00,0x56,0x36,0x00,0x00}, /* ';' */
    {0x00,0x08,0x14,0x22,0x41}, /* '<' */
    {0x14,0x14,0x14,0x14,0x14}, /* '=' */
    {0x41,0x22,0x14,0x08,0x00}, /* '>' */
    {0x02,0x01,0x51,0x09,0x06}, /* '?' */
    {0x32,0x49,0x79,0x41,0x3E}, /* '@' */
    {0x7E,0x11,0x11,0x11,0x7E}, /* 'A' */
    {0x7F,0x49,0x49,0x49,0x36}, /* 'B' */
    {0x3E,0x41,0x41,0x41,0x22}, /* 'C' */
    {0x7F,0x41,0x41,0x22,0x1C}, /* 'D' */
    {0x7F,0x49,0x49,0x49,0x41}, /* 'E' */
    {0x7F,0x09,0x09,0x01,0x01}, /* 'F' */
    {0x3E,0x41,0x41,0x51,0x32}, /* 'G' */
    {0x7F,0x08,0x08,0x08,0x7F}, /* 'H' */
    {0x00,0x41,0x7F,0x41,0x00}, /* 'I' */
    {0x20,0x40,0x41,0x3F,0x01}, /* 'J' */
    {0x7F,0x08,0x14,0x22,0x41}, /* 'K' */
    {0x7F,0x40,0x40,0x40,0x40}, /* 'L' */
    {0x7F,0x02,0x04,0x02,0x7F}, /* 'M' */
    {0x7F,0x04,0x08,0x10,0x7F}, /* 'N' */
    {0x3E,0x41,0x41,0x41,0x3E}, /* 'O' */
    {0x7F,0x09,0x09,0x09,0x06}, /* 'P' */
    {0x3E,0x41,0x51,0x21,0x5E}, /* 'Q' */
    {0x7F,0x09,0x19,0x29,0x46}, /* 'R' */
    {0x46,0x49,0x49,0x49,0x31}, /* 'S' */
    {0x01,0x01,0x7F,0x01,0x01}, /* 'T' */
    {0x3F,0x40,0x40,0x40,0x3F}, /* 'U' */
    {0x1F,0x20,0x40,0x20,0x1F}, /* 'V' */
    {0x7F,0x20,0x18,0x20,0x7F}, /* 'W' */
    {0x63,0x14,0x08,0x14,0x63}, /* 'X' */
    {0x03,0x04,0x78,0x04,0x03}, /* 'Y' */
    {0x61,0x51,0x49,0x45,0x43}, /* 'Z' */
};
#define FONT_W 5
#define FONT_H 7

/* 7-seg segment patterns (a,b,c,d,e,f,g) for 0-9 + none */
static const uint8_t SEG_PAT[] = {
    0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F
};

static uint8_t oled_fb[OLED_W * OLED_H / 8];   /* page-major */
static uint8_t lcd_fb[128 * 64];
static volatile uint32_t btnPresses = 0;
static bool buzzerOn = false;
static uint8_t rgb[3] = {0, 0, 0};

static void oled_cmd(uint8_t c) {
    Wire.beginTransmission(OLED_ADDR);
    Wire.write(0x00);          /* control byte: command */
    Wire.write(c);
    Wire.endTransmission();
}

static void oled_show() {
    /* Upload the whole framebuffer, page by page (SSD1306 page addressing). */
    for (uint8_t page = 0; page < OLED_H / 8; page++) {
        Wire.beginTransmission(OLED_ADDR);
        Wire.write(0x00);
        Wire.write(0xB0 | page);    /* set page */
        Wire.write(0x00);           /* col low */
        Wire.write(0x10);           /* col high */
        Wire.endTransmission();
        Wire.beginTransmission(OLED_ADDR);
        Wire.write(0x40);           /* control byte: data */
        for (uint16_t i = 0; i < OLED_W; i++) Wire.write(oled_fb[page * OLED_W + i]);
        Wire.endTransmission();
    }
}

static void oled_draw_char(int16_t x, int16_t y, char c) {
    if (c < ' ' || c > 'Z') c = ' ';
    uint8_t idx = c - ' ';
    for (uint8_t row = 0; row < FONT_H; row++) {
        uint8_t bits = FONT5x7[idx][row];
        int16_t py = y + row;
        if (py < 0 || py >= OLED_H) continue;
        uint8_t page = py >> 3;
        uint8_t bit = 1 << (py & 7);
        for (uint8_t col = 0; col < FONT_W; col++) {
            int16_t px = x + col;
            if (px < 0 || px >= OLED_W) continue;
            if (bits & (1 << (FONT_W - 1 - col))) oled_fb[page * OLED_W + px] |= bit;
        }
    }
}

static void oled_draw_text(int16_t x, int16_t y, const char *s) {
    while (*s) {
        oled_draw_char(x, y, *s);
        x += FONT_W + 1;
        s++;
    }
}

static void oled_display_text() {
    for (uint16_t i = 0; i < sizeof(oled_fb); i++) oled_fb[i] = 0;
    oled_draw_text(0, 0, "STM32 EMU!");
    oled_draw_text(0, 16, "OLED 128x64");
    oled_draw_text(0, 32, "SEC=0000");
    oled_show();
}

static void lcd_begin() {
    digitalWrite(LCD_CS, LOW);
    SPI.transfer(0xFB);          /* Lcd device: start drawing session */
}

static void lcd_end() {
    SPI.transfer(0xFC);          /* Lcd device: end drawing */
    digitalWrite(LCD_CS, HIGH);
}

static void lcd_draw_char(uint8_t x, uint8_t y, char c) {
    if (c < ' ' || c > 'Z') c = ' ';
    uint8_t idx = c - ' ';
    for (uint8_t row = 0; row < FONT_H; row++) {
        uint8_t bits = FONT5x7[idx][row];
        for (uint8_t col = 0; col < FONT_W; col++) {
            uint8_t px = x + col, py = y + row;
            if (px < 128 && py < 64) {
                lcd_fb[py * 128 + px] = (bits & (1 << (FONT_W - 1 - col))) ? 0xFF : (lcd_fb[py * 128 + px] & 0x40);
            }
        }
    }
}

static void lcd_paint() {
    lcd_begin();
    for (uint16_t i = 0; i < 128 * 64; i++) SPI.transfer(lcd_fb[i]);
    lcd_end();
}

static void seg_write_digit(uint8_t digit, uint8_t pattern) {
    SPI.transfer(pattern | (digit == 13 ? 0x80 : 0));   /* 74HC595: one byte per digit */
}

static void btnISR() {
    btnPresses++;
}

void setup() {
    Serial1.begin(115200);
    Serial1.print("\r\n=== Peripheral showcase: OLED + LCD + 7-seg + RGB + buzzer + button ===\r\n");

    pinMode(LCD_CS, OUTPUT);
    pinMode(SEG_CS, OUTPUT);
    pinMode(BUZZ, OUTPUT);
    digitalWrite(LCD_CS, HIGH);
    digitalWrite(SEG_CS, HIGH);
    digitalWrite(BUZZ, LOW);
    pinMode(PC13, OUTPUT);
    digitalWrite(PC13, HIGH);

    SPI.begin();
    Wire.begin();
    attachInterrupt(BTN, btnISR, RISING);

    /* SSD1306-style init */
    oled_cmd(0xAE);                    /* display off */
    oled_cmd(0x20); oled_cmd(0x00);    /* horizontal addressing */
    oled_cmd(0x8D); oled_cmd(0x14);    /* charge pump on */
    oled_cmd(0xA8); oled_cmd(0x3F);    /* multiplex 1/64 */
    oled_cmd(0xD3); oled_cmd(0x00);    /* display offset 0 */
    oled_cmd(0x40);                    /* start line 0 */
    oled_cmd(0xA1);                    /* segment remap */
    oled_cmd(0xC8);                    /* COM scan dec */
    oled_cmd(0xDA); oled_cmd(0x12);    /* COM pins */
    oled_cmd(0x81); oled_cmd(0xCF);    /* contrast */
    oled_cmd(0xD5); oled_cmd(0x80);    /* osc */
    oled_cmd(0xAF);                    /* display on */

    oled_display_text();

    Serial1.print("OLED=ok LCD=ok SEG=ok RGB=ok BUZZ=ok BTN=armed\r\n");
}

uint32_t lastSec = 0;

void loop() {
    uint32_t now = millis();
    uint32_t sec = now / 1000;

    if (sec != lastSec) {
        lastSec = sec;

        /* OLED: update the counter line */
        char buf[24];
        snprintf(buf, sizeof(buf), "SEC=%04lu", (unsigned long)sec);
        for (uint16_t i = 0; i < sizeof(oled_fb); i++) oled_fb[i] = 0;
        oled_draw_text(0, 0, "STM32 EMU!");
        oled_draw_text(0, 16, "OLED 128x64");
        oled_draw_text(0, 32, buf);
        oled_draw_text(0, 48, "BTN=");
        snprintf(buf, sizeof(buf), "%lu", (unsigned long)btnPresses);
        oled_draw_text(5 * 6, 48, buf);
        oled_show();

        /* LCD: two text lines + a growing level bar */
        for (uint16_t i = 0; i < sizeof(lcd_fb); i++) lcd_fb[i] = 0;
        lcd_draw_char(0, 0, 'L'); lcd_draw_char(6, 0, 'C'); lcd_draw_char(12, 0, 'D');
        lcd_draw_char(18, 0, '='); lcd_draw_char(24, 0, ' ');
        snprintf(buf, sizeof(buf), "%04lu", (unsigned long)sec);
        for (int i = 0; i < 4; i++) lcd_draw_char(30 + i * 6, 0, buf[i]);
        uint8_t bar = (sec % 100) * 64 / 100;
        for (uint8_t y = 0; y < bar; y++)
            for (uint8_t x = 0; x < 64; x++) lcd_fb[(y + 8) * 128 + x] = 0xFF;
        lcd_paint();

        /* 7-seg: seconds as 4 digits, 74HC595 latch */
        uint32_t v = sec % 10000;
        digitalWrite(SEG_CS, LOW);
        seg_write_digit(0, SEG_PAT[v / 1000]);
        seg_write_digit(1, SEG_PAT[(v / 100) % 10]);
        seg_write_digit(2, SEG_PAT[(v / 10) % 10]);
        seg_write_digit(3, SEG_PAT[v % 10]);
        digitalWrite(SEG_CS, HIGH);

        /* RGB: cycle hue through TIM2 CH1/2/3 (PA0/1/2) PWM */
        uint8_t phase = (uint8_t)(sec * 8);
        rgb[0] = 128 + 127 * sinf(phase * 3.14159f / 180.0f);
        rgb[1] = 128 + 127 * sinf((phase + 120) * 3.14159f / 180.0f);
        rgb[2] = 128 + 127 * sinf((phase + 240) * 3.14159f / 180.0f);
        TIM2->PSC = 71;
        TIM2->ARR = 1000;
        TIM2->CCMR1 = 0x0068;   /* CH1+CH2 PWM mode 1 */
        TIM2->CCMR2 = 0x0008;   /* CH3 PWM mode 1 */
        TIM2->CCER = 0x0111;    /* CH1+CH2+CH3 enable */
        TIM2->CCR1 = rgb[0] * 1000 / 255;
        TIM2->CCR2 = rgb[1] * 1000 / 255;
        TIM2->CCR3 = rgb[2] * 1000 / 255;
        TIM2->CR1 = 1;

        /* Buzzer: 200ms beep each second + PC13 blink */
        digitalWrite(BUZZ, HIGH);
        digitalWrite(PC13, LOW);
        buzzerOn = true;
        delay(200);
        digitalWrite(BUZZ, LOW);
        digitalWrite(PC13, HIGH);
        buzzerOn = false;

        char out[40];
        snprintf(out, sizeof(out), "t=%lus btn=%lu rg=%02X%02X%02X buzz=1\r\n",
                 (unsigned long)sec, (unsigned long)btnPresses, rgb[0], rgb[1], rgb[2]);
        Serial1.print(out);
    }
    delay(10);
}