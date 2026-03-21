#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

static volatile int foo[4] = {0, 0, 0, 0};

static void cleanup1(void *arg) { (void)arg; foo[0] = 1; }
static void cleanup2(void *arg) { (void)arg; foo[1] = 2; }
static void cleanup3(void *arg) { (void)arg; foo[2] = 3; }
static void cleanup4(void *arg) { (void)arg; foo[3] = 4; }

static void *worker(void *arg) {
    (void)arg;
    pthread_setcancelstate(PTHREAD_CANCEL_ENABLE, NULL);
    pthread_setcanceltype(PTHREAD_CANCEL_DEFERRED, NULL);

    pthread_cleanup_push(cleanup1, NULL);
    pthread_cleanup_push(cleanup2, NULL);
    pthread_cleanup_push(cleanup3, NULL);
    pthread_cleanup_push(cleanup4, NULL);

    for (;;) {
        pthread_testcancel();
        usleep(1000);
    }

    pthread_cleanup_pop(0);
    pthread_cleanup_pop(0);
    pthread_cleanup_pop(0);
    pthread_cleanup_pop(0);
    return NULL;
}

int main(void) {
    pthread_t th;
    void *res = NULL;

    if (pthread_create(&th, NULL, worker, NULL) != 0) {
        perror("pthread_create");
        return 2;
    }

    usleep(20000);
    if (pthread_cancel(th) != 0) {
        perror("pthread_cancel");
        return 3;
    }

    if (pthread_join(th, &res) != 0) {
        perror("pthread_join");
        return 4;
    }

    printf("join res=%p expected=%p\n", res, PTHREAD_CANCELED);
    printf("foo={%d,%d,%d,%d}\n", foo[0], foo[1], foo[2], foo[3]);

    if (res != PTHREAD_CANCELED) return 10;
    if (foo[0] != 1 || foo[1] != 2 || foo[2] != 3 || foo[3] != 4) return 11;

    puts("pthread_cancel_small: PASS");
    return 0;
}
