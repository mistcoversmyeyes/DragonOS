#include <stdio.h>
#include <unistd.h>
#include <sys/syscall.h>

int main() {
    printf("Testing custom syscall 2333...\n");
    
    // Call our custom syscall 2333
    long result = syscall(2333);
    
    printf("Successfully called syscall 2333, return value is: %ld\n", result);
    
    return 0;
}