// STM32 Bluepill WASM emulator demo: push-button stopwatch.
//
// PB13 (the Blue Pill button footprint, EXTI13 falling edge) toggles
// run/stop; TIM2 free-runs at 1MHz in emulator time and the elapsed time
// prints on every state change and every second while running. On the page,
// press the button widget; headless, drive PB13 via gpioSetInput.
// malloc-free (no String/new).
#include <Arduino.h>
#include <HardwareTimer.h>

HardwareTimer timer2(TIM2);
volatile bool running = false;
volatile bool changed = false;
volatile uint32_t ticks_at_toggle = 0;

void onButton() {
    running = !running;
    changed = true;
    ticks_at_toggle = TIM2->CNT;
}

static void print_elapsed(uint32_t us) {
    Serial.print("t=");
    Serial.print(us / 1000000u);
    Serial.print('.');
    uint32_t frac = (us % 1000000u) / 1000u;
    if (frac < 100) Serial.print('0');
    if (frac < 10) Serial.print('0');
    Serial.print(frac);
    Serial.println('s');
}

void setup() {
    pinMode(PC13, OUTPUT);
    pinMode(PB13, INPUT_PULLUP);
    attachInterrupt(digitalPinToInterrupt(PB13), onButton, FALLING);
    Serial.begin(115200);
    Serial.println("\r\n=== Stopwatch demo (PB13 button + TIM2) ===");
    Serial.println("press the button: run/stop, prints elapsed");

    timer2.setPrescaleFactor(72);   // 1MHz at 72MHz: 1 tick = 1us
    timer2.setOverflow(0xFFFFFFFF, TICK_FORMAT);
    timer2.resume();
}

void loop() {
    static uint32_t last = 0;
    static uint32_t base = 0;      // CNT value at last (re)start
    static uint32_t acc = 0;       // accumulated us while stopped
    uint32_t now = TIM2->CNT;
    if (changed) {
        changed = false;
        if (running) {
            base = now;
            Serial.println("run");
        } else {
            acc += now - base;
            Serial.print("stop ");
            print_elapsed(acc);
        }
        digitalWrite(PC13, running ? LOW : HIGH);
    }
    if (running && now - last > 1000000u) {
        last = now;
        print_elapsed(acc + (now - base));
    }
}
