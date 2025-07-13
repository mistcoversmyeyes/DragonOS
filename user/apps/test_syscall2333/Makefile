ifeq ($(ARCH), x86_64)
	CROSS_COMPILE=x86_64-linux-musl-
else ifeq ($(ARCH), riscv64)
	CROSS_COMPILE=riscv64-linux-musl-
else ifeq ($(ARCH), loongarch64)
	CROSS_COMPILE=loongarch64-unknown-linux-gnu-
endif

CC=$(CROSS_COMPILE)gcc

.PHONY: all
all: main.c
	$(CC) -static -o test_syscall2333 main.c

.PHONY: install clean
install: all
	mv test_syscall2333 $(DADK_CURRENT_BUILD_DIR)/test_syscall2333

clean:
	rm test_syscall2333 *.o

