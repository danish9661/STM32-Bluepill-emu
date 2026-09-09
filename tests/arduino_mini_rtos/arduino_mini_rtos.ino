// STM32 Bluepill WASM emulator demo: preemptive mini-RTOS (2 tasks).
//
// Hand-rolled kernel proving the modeled primitives: PSP thread stacks,
// PendSV context switch (r4-r11 save/restore + EXC_RETURN reshaping), a
// 1ms TIM7 tick (HardwareTimer) that pends PendSV, and LDREX/STREX
// spinlock prints. Task A/B each print seq lines every 200 ticks; the
// sequences must be gap-free (any register/stack corruption garbles
// them). malloc-free (no String/new).
#include <Arduino.h>
#include <HardwareTimer.h>

#define SCB_ICSR (*(volatile uint32_t *)0xE000ED04u)
#define PENDSVSET (1u << 28)

typedef struct { uint32_t *sp; } tcb_t;

static uint32_t stackA[256], stackB[256];
extern "C" {
tcb_t tcbs[2];
volatile uint32_t cur_task = 0;
volatile uint32_t sched_started = 0;
}
static volatile uint32_t ticks = 0;
static volatile int uart_lock = 0;

static void task_init(tcb_t *t, uint32_t *stack, void (*fn)(void)) {
    uint32_t *top = stack + 256 - 16; // hw frame (8) + r4-r11 slots (8)
    top[8] = 0; top[9] = 0; top[10] = 0; top[11] = 0; // R0-R3
    top[12] = 0;                                      // R12
    top[13] = 0xFFFFFFFD;                             // LR (thread PSP)
    top[14] = (uint32_t)fn;                           // PC
    top[15] = 0x01000000;                             // xPSR (Thumb)
    t->sp = top; // switch-in restores r4-r11 from here, then hw frame
}

extern "C" __attribute__((naked)) void PendSV_Handler(void) {
    __asm volatile(
        "ldr r3, =sched_started\n"
        "ldr r2, [r3]\n"
        "cmp r2, #0\n"
        "bne 1f\n"
        "movs r2, #1\n"
        "str r2, [r3]\n"          // started = 1 (no context to save yet)
        "ldr r3, =tcbs\n"
        "ldr r0, [r3]\n"          // task0 sp
        "b 2f\n"
        "1:\n"
        "mrs r0, psp\n"
        "stmdb r0!, {r4-r11}\n"   // save outgoing context
        "ldr r3, =cur_task\n"
        "ldr r2, [r3]\n"
        "ldr r1, =tcbs\n"
        "lsls r2, r2, #2\n"
        "str r0, [r1, r2]\n"      // tcbs[cur].sp = sp
        "ldr r2, [r3]\n"
        "eors r2, r2, #1\n"       // round-robin 2 tasks
        "str r2, [r3]\n"
        "lsls r2, r2, #2\n"
        "ldr r0, [r1, r2]\n"      // incoming sp
        "2:\n"
        "ldmia r0!, {r4-r11}\n"   // restore incoming context
        "msr psp, r0\n"
        "mrs r2, control\n"
        "orrs r2, #2\n"
        "msr control, r2\n"       // thread uses PSP from here on
        "orr lr, lr, #4\n"        // EXC_RETURN -> thread-PSP (0xFFFFFFFD)
        "bx lr\n"
    );
}

static void onTick(void) {
    ticks++;
    SCB_ICSR |= PENDSVSET;
}

static void task_body(char id) {
    uint32_t last = 0, seq = 0;
    for (;;) {
        uint32_t now = ticks;
        if (now - last >= 200) {
            last = now;
            while (__sync_lock_test_and_set(&uart_lock, 1)) {}
            Serial.print(id);
            Serial.print(seq);
            Serial.print(" t=");
            Serial.println(now);
            __sync_lock_release(&uart_lock);
            seq++;
        }
    }
}

static void taskA(void) { task_body('A'); }
static void taskB(void) { task_body('B'); }

HardwareTimer tickTimer(TIM4);

void setup() {
    pinMode(PC13, OUTPUT);
    Serial.begin(115200);
    Serial.println("\r\n=== mini-RTOS demo (PendSV + PSP, 2 preemptive tasks) ===");
    task_init(&tcbs[0], stackA, taskA);
    task_init(&tcbs[1], stackB, taskB);
    NVIC_SetPriority(PendSV_IRQn, 15); // PendSV lowest: never nests ISRs
    tickTimer.setOverflow(1000, MICROSEC_FORMAT); // 1 ms tick
    tickTimer.attachInterrupt(onTick);
    tickTimer.resume();
    Serial.println("scheduler live");
}

void loop() {
    for (;;) {} // main never resumes after the first PendSV switch
}
