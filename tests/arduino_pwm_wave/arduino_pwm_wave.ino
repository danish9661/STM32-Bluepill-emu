// STM32 Bluepill WASM emulator demo: TIM DMA-burst PWM wave on TIM3 CH1 (PA6).
//
// Register-level setup (no HAL): TIM3 runs PWM (ARR=999) with UDE set and
// DCR programmed for a single-register burst (DBA=CCR1, DBL=0). Each frame
// the CPU stages one duty value and re-arms DMA1 CH3 (mem->TIM3_DMAR,
// CNDTR=1); the next update event DMAs it into CCR1 with zero CPU in the
// transfer path. Duty follows an 8-step triangle; each step prints
// duty=<staged>  ccr=<readback>. malloc-free (no String/new).
#include <Arduino.h>

#define TIM3_BASE 0x40000400u
#define TIM3_DMAR (TIM3_BASE + 0x4Cu)
#define DMA1_BASE 0x40020000u
#define DMA1_CH3 (DMA1_BASE + 0x30u)  // CCR/CNDTR/CPAR/CMAR at +0x0/4/8/C

static volatile uint16_t burst_duty = 0;
static const uint16_t WAVE[8] = { 0, 143, 286, 429, 571, 714, 857, 999 };

static inline void dma_ch3_write(uint32_t off, uint32_t v) {
    *(volatile uint32_t *)(DMA1_CH3 + off) = v;
}

static void dma_arm(uint16_t duty) {
    burst_duty = duty;
    dma_ch3_write(0x00, 0);                    // EN=0 while programming
    dma_ch3_write(0x04, 1);                    // CNDTR=1: one burst element
    dma_ch3_write(0x08, TIM3_DMAR);            // CPAR: burst register
    dma_ch3_write(0x0C, (uint32_t)&burst_duty);// CMAR: staged value
    dma_ch3_write(0x00, 0x511);                // EN + DIR + PSIZE/MSIZE=16-bit
}

void setup() {
    pinMode(PC13, OUTPUT);
    Serial.begin(115200);
    Serial.println("\r\n=== TIM DMA-burst PWM wave (TIM3 CH1/PA6 + DMA1 CH3) ===");
    Serial.println("duty=<staged>  ccr=<readback>");

    RCC->APB1ENR |= RCC_APB1ENR_TIM3EN;
    RCC->AHBENR |= RCC_AHBENR_DMA1EN;
    RCC->APB2ENR |= RCC_APB2ENR_IOPAEN | RCC_APB2ENR_AFIOEN;
    GPIOA->CRL = (GPIOA->CRL & ~(0xFu << 24)) | (0xBu << 24);  // PA6: AF push-pull
    TIM3->PSC = 71;       // 1 tick per 72 instructions
    TIM3->ARR = 999;      // 1kHz PWM
    TIM3->CCMR1 = 0x0068; // CH1: PWM mode 1 + preload
    TIM3->CCER = 0x0001;  // CH1 enable
    TIM3->CCR1 = 0;
    TIM3->DCR = (0 << 8) | 0x0D;  // DBL=0 (1 transfer), DBA=CCR1
    TIM3->DIER = (1 << 8);        // UDE: update -> DMA1 CH3 request
    TIM3->CR1 = 1;                // CEN
    dma_arm(WAVE[0]);
}

void loop() {
    static uint32_t frame = 0;
    static uint32_t last = 0;
    uint32_t now = millis();
    if (now - last >= 100) {  // one wave step per 100ms
        last = now;
        uint16_t duty = WAVE[frame % 8];
        dma_arm(duty);
        delay(2);  // let several update events (72K instr each) run the burst
        Serial.print("duty=");
        Serial.print(duty);
        Serial.print("  ccr=");
        Serial.println(TIM3->CCR1);
        frame++;
    }
}
