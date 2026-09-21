#define _GNU_SOURCE
#include <stdio.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/select.h>
#include <sys/syscall.h>
#include <sys/time.h>
#include <unistd.h>

static int normalized(const char *name, const struct timeval *tv) {
  if (tv->tv_sec < 0 || tv->tv_usec < 0 || tv->tv_usec >= 1000000) {
    fprintf(stderr, "FAIL %s: sec=%lld usec=%lld\n", name,
            (long long)tv->tv_sec, (long long)tv->tv_usec);
    return 0;
  }
  return 1;
}

int main(void) {
  struct timeval tv;
  struct rusage usage;
  for (int raw = 0; raw < 2; ++raw) {
    for (int i = 0; i < 10000; ++i) {
      /* Match the mixed syscall traffic around lmbench's timing reads. */
      if (getrusage(RUSAGE_SELF, &usage) != 0) {
        perror("getrusage");
        return 1;
      }
      (void)getppid();
      memset(&tv, 0xa5, sizeof(tv));
      long rc = raw ? syscall(SYS_gettimeofday, &tv, NULL)
                    : gettimeofday(&tv, NULL);
      if (rc != 0) {
        perror("gettimeofday");
        return 1;
      }
      if (!normalized(raw ? "raw gettimeofday" : "libc gettimeofday", &tv))
        return 1;
    }
  }

  struct itimerval timer = {{0, 123456}, {60, 654321}};
  struct itimerval current, old, disabled = {0};
  if (setitimer(ITIMER_REAL, &timer, NULL) != 0 ||
      getitimer(ITIMER_REAL, &current) != 0 ||
      setitimer(ITIMER_REAL, &disabled, &old) != 0) {
    perror("itimer");
    return 1;
  }
  if (!normalized("getitimer interval", &current.it_interval) ||
      !normalized("getitimer value", &current.it_value) ||
      !normalized("setitimer old interval", &old.it_interval) ||
      !normalized("setitimer old value", &old.it_value) ||
      current.it_interval.tv_sec != 0 || current.it_interval.tv_usec != 123456 ||
      old.it_interval.tv_sec != 0 || old.it_interval.tv_usec != 123456) {
    fprintf(stderr, "FAIL itimer timeval round trip\n");
    return 1;
  }

#ifdef SYS_select
  int pipefd[2];
  if (pipe(pipefd) != 0 || write(pipefd[1], "x", 1) != 1) {
    perror("pipe");
    return 1;
  }
  fd_set ready;
  FD_ZERO(&ready);
  FD_SET(pipefd[0], &ready);
  tv.tv_sec = 1;
  tv.tv_usec = 123456;
  /* libc may implement select via pselect6; exercise timeval copyback directly. */
  long rc = syscall(SYS_select, pipefd[0] + 1, &ready, NULL, NULL, &tv);
  close(pipefd[0]);
  close(pipefd[1]);
  if (rc != 1 || !normalized("select remaining timeout", &tv)) {
    fprintf(stderr, "FAIL select: rc=%ld\n", rc);
    return 1;
  }
#endif
  puts("PASS native timeval ABI");
  return 0;
}
