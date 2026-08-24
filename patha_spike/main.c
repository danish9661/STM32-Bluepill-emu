#include <stdio.h>
extern int spike_add(int a, int b);
extern int spike_multiply(int a, int b);
int main() {
    int s = spike_add(3, 4);
    int p = spike_multiply(3, 4);
    printf("spike_add(3,4)=%d spike_multiply(3,4)=%d\n", s, p);
    if (s == 7 && p == 12) { printf("PATHA_LINK_OK\n"); return 0; }
    return 1;
}
