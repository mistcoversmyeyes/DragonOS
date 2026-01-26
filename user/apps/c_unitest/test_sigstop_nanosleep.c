#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static void print_ts(const char *tag, const struct timespec *ts) {
    printf("%s: %lld.%09ld\n", tag, (long long)ts->tv_sec, ts->tv_nsec);
}

static struct timespec ts_diff(const struct timespec *start,
                               const struct timespec *end) {
    struct timespec out;
    out.tv_sec = end->tv_sec - start->tv_sec;
    out.tv_nsec = end->tv_nsec - start->tv_nsec;
    if (out.tv_nsec < 0) {
        out.tv_sec -= 1;
        out.tv_nsec += 1000000000L;
    }
    return out;
}

int main(void) {
    printf("sigstop_nanosleep_diag\n");

    pid_t pid = fork();
    if (pid < 0) {
        perror("fork");
        return 1;
    }

    if (pid == 0) {
        struct timespec start = {0}, end = {0};
        struct timespec req = {15, 0};
        struct timespec rem = {0, 0};

        if (clock_gettime(CLOCK_MONOTONIC, &start) != 0) {
            perror("clock_gettime(start)");
            _exit(1);
        }

        int ret = nanosleep(&req, &rem);
        int saved_errno = errno;

        if (clock_gettime(CLOCK_MONOTONIC, &end) != 0) {
            perror("clock_gettime(end)");
            _exit(1);
        }

        struct timespec delta = ts_diff(&start, &end);
        print_ts("start", &start);
        print_ts("finish", &end);
        print_ts("delta", &delta);
        printf("delta_seconds: %.9f\n",
               (double)delta.tv_sec + (double)delta.tv_nsec / 1e9);

        if (ret != 0) {
            printf("nanosleep: ret=%d errno=%d (%s)\n", ret, saved_errno,
                   strerror(saved_errno));
            printf("remaining: %lld.%09ld\n", (long long)rem.tv_sec, rem.tv_nsec);
        } else {
            printf("nanosleep: ret=0\n");
        }

        _exit(ret == 0 ? 0 : 2);
    }

    sleep(5);
    if (kill(pid, SIGSTOP) != 0) {
        perror("kill(SIGSTOP)");
    } else {
        printf("parent: SIGSTOP sent\n");
    }

    int status = 0;
    if (waitpid(pid, &status, WUNTRACED) < 0) {
        perror("waitpid(WUNTRACED)");
    } else if (WIFSTOPPED(status)) {
        printf("parent: child stopped by signal %d\n", WSTOPSIG(status));
    } else {
        printf("parent: waitpid(WUNTRACED) status=0x%x\n", status);
    }

    sleep(5);
    if (kill(pid, SIGCONT) != 0) {
        perror("kill(SIGCONT)");
    } else {
        printf("parent: SIGCONT sent\n");
    }

#ifdef WCONTINUED
    if (waitpid(pid, &status, WCONTINUED) < 0) {
        perror("waitpid(WCONTINUED)");
    } else if (WIFCONTINUED(status)) {
        printf("parent: child continued\n");
    } else {
        printf("parent: waitpid(WCONTINUED) status=0x%x\n", status);
    }
#endif

    if (waitpid(pid, &status, 0) < 0) {
        perror("waitpid(EXIT)");
    } else if (WIFEXITED(status)) {
        printf("parent: child exited with code %d\n", WEXITSTATUS(status));
    } else if (WIFSIGNALED(status)) {
        printf("parent: child killed by signal %d\n", WTERMSIG(status));
    } else {
        printf("parent: final wait status=0x%x\n", status);
    }

    return 0;
}
