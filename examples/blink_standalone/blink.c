void SystemInit(void) {}
void _exit(int status) { while (1); }
int _kill(int pid, int sig) { return -1; }
int _getpid(void) { return 1; }
void _init(void) {}
void _fini(void) {}

void setup(void) {
    *(volatile unsigned int *)0x40021018 |= (1 << 4);
    unsigned int crh = *(volatile unsigned int *)0x40011004;
    crh = (crh & ~(0xF << 20)) | (0x3 << 20);
    *(volatile unsigned int *)0x40011004 = crh;
}

void loop(void) {
    volatile unsigned int *bsrr = (volatile unsigned int *)0x40011010;
    *bsrr = (1 << 13);
    for (volatile int i = 0; i < 200000; i++);
    *bsrr = (1 << 29);
    for (volatile int i = 0; i < 200000; i++);
}

int main(void) {
    setup();
    while (1) { loop(); }
}
