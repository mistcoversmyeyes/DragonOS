#include <stdio.h>
#include <stdlib.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>
#include <errno.h>
#include <string.h>

#define TEST_ITERATIONS 100
#define SLEEP_US 100000  // 100ms

// 测试 gettimeofday 系统调用
void test_gettimeofday_basic() {
    printf("=== Testing gettimeofday() basic functionality ===\n");

    struct timeval tv;
    int errors = 0;

    for (int i = 0; i < TEST_ITERATIONS; i++) {
        if (gettimeofday(&tv, NULL) != 0) {
            printf("Error: gettimeofday failed at iteration %d: %s\n",
                   i, strerror(errno));
            errors++;
            continue;
        }

        // 检查时间值是否合理
        if (tv.tv_sec < 0) {
            printf("Warning: tv_sec negative at iteration %d: %ld\n", i, tv.tv_sec);
        }

        if (tv.tv_usec < 0 || tv.tv_usec >= 1000000) {
            printf("Warning: tv_usec out of range at iteration %d: %ld\n",
                   i, tv.tv_usec);
        }

        if (i % 10 == 0) {
            printf("Iteration %3d: tv_sec = %10ld, tv_usec = %06ld\n",
                   i, tv.tv_sec, tv.tv_usec);
        }

        usleep(SLEEP_US);
    }

    printf("gettimeofday test completed with %d errors\n\n", errors);
}

// 测试时间单调递增
void test_time_monotonic() {
    printf("=== Testing time monotonicity ===\n");

    struct timeval prev_tv, curr_tv;
    int regressions = 0;

    if (gettimeofday(&prev_tv, NULL) != 0) {
        printf("Error: Initial gettimeofday failed\n");
        return;
    }

    for (int i = 0; i < TEST_ITERATIONS; i++) {
        if (gettimeofday(&curr_tv, NULL) != 0) {
            printf("Error: gettimeofday failed at iteration %d\n", i);
            continue;
        }

        // 检查时间是否倒退
        if (curr_tv.tv_sec < prev_tv.tv_sec ||
            (curr_tv.tv_sec == prev_tv.tv_sec && curr_tv.tv_usec < prev_tv.tv_usec)) {
            printf("Time regression at iteration %d: %ld.%06ld -> %ld.%06ld\n",
                   i, prev_tv.tv_sec, prev_tv.tv_usec,
                   curr_tv.tv_sec, curr_tv.tv_usec);
            regressions++;
        }

        prev_tv = curr_tv;
        usleep(SLEEP_US);
    }

    printf("Time monotonicity test completed with %d regressions\n\n", regressions);
}

// 测试 usleep 精度
void test_usleep_accuracy() {
    printf("=== Testing usleep() accuracy ===\n");

    struct timeval start, end;
    long total_elapsed_us = 0;
    int large_errors = 0;

    for (int i = 0; i < TEST_ITERATIONS; i++) {
        if (gettimeofday(&start, NULL) != 0) {
            printf("Error: gettimeofday failed at start of iteration %d\n", i);
            continue;
        }

        usleep(SLEEP_US);

        if (gettimeofday(&end, NULL) != 0) {
            printf("Error: gettimeofday failed at end of iteration %d\n", i);
            continue;
        }

        long elapsed_us = (end.tv_sec - start.tv_sec) * 1000000L +
                         (end.tv_usec - start.tv_usec);
        total_elapsed_us += elapsed_us;

        // 检查睡眠时间是否在合理范围内（允许 ±20% 误差）
        if (elapsed_us < SLEEP_US * 0.8 || elapsed_us > SLEEP_US * 1.2) {
            printf("Large sleep error at iteration %d: requested %dus, got %ldus\n",
                   i, SLEEP_US, elapsed_us);
            large_errors++;
        }

        if (i % 20 == 0) {
            printf("Iteration %3d: sleep requested %dus, actual %ldus\n",
                   i, SLEEP_US, elapsed_us);
        }
    }

    double avg_elapsed_us = (double)total_elapsed_us / TEST_ITERATIONS;
    printf("Average sleep time: %.2fus (requested %dus)\n", avg_elapsed_us, SLEEP_US);
    printf("usleep accuracy test completed with %d large errors\n\n", large_errors);
}

// 测试连续快速调用
void test_rapid_calls() {
    printf("=== Testing rapid gettimeofday() calls ===\n");

    struct timeval tv;
    int calls = 1000;
    int errors = 0;

    for (int i = 0; i < calls; i++) {
        if (gettimeofday(&tv, NULL) != 0) {
            errors++;
        }

        // 检查时间值是否合理
        if (tv.tv_usec < 0 || tv.tv_usec >= 1000000) {
            printf("Warning: Invalid tv_usec during rapid call %d: %ld\n",
                   i, tv.tv_usec);
        }
    }

    printf("Rapid calls test: %d calls, %d errors\n\n", calls, errors);
}

// 测试 clock_gettime (如果可用)
void test_clock_gettime() {
    printf("=== Testing clock_gettime() ===\n");

#ifdef _POSIX_TIMERS
    struct timespec ts;
    int errors = 0;

    // 测试 CLOCK_REALTIME
    for (int i = 0; i < TEST_ITERATIONS / 2; i++) {
        if (clock_gettime(CLOCK_REALTIME, &ts) != 0) {
            printf("Error: clock_gettime(CLOCK_REALTIME) failed: %s\n",
                   strerror(errno));
            errors++;
            continue;
        }

        if (i % 10 == 0) {
            printf("CLOCK_REALTIME %3d: sec = %10ld, nsec = %09ld\n",
                   i, ts.tv_sec, ts.tv_nsec);
        }

        usleep(SLEEP_US);
    }

    // 测试 CLOCK_MONOTONIC
    for (int i = 0; i < TEST_ITERATIONS / 2; i++) {
        if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) {
            printf("Error: clock_gettime(CLOCK_MONOTONIC) failed: %s\n",
                   strerror(errno));
            errors++;
            continue;
        }

        if (i % 10 == 0) {
            printf("CLOCK_MONOTONIC %3d: sec = %10ld, nsec = %09ld\n",
                   i, ts.tv_sec, ts.tv_nsec);
        }

        usleep(SLEEP_US);
    }

    printf("clock_gettime test completed with %d errors\n\n", errors);
#else
    printf("clock_gettime not available (_POSIX_TIMERS not defined)\n\n");
#endif
}

int main() {
    printf("========================================\n");
    printf("Time System Test Application\n");
    printf("Testing DragonOS time subsystem\n");
    printf("========================================\n\n");

    // 运行所有测试
    test_gettimeofday_basic();
    test_time_monotonic();
    test_usleep_accuracy();
    test_rapid_calls();
    test_clock_gettime();

    printf("========================================\n");
    printf("All tests completed\n");
    printf("========================================\n");

    return 0;
}