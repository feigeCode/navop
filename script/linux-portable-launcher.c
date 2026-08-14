#define _GNU_SOURCE

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#if defined(__aarch64__)
#define NAVOP_PORTABLE_LOADER "ld-linux-aarch64.so.1"
#elif defined(__x86_64__)
#define NAVOP_PORTABLE_LOADER "ld-linux-x86-64.so.2"
#else
#error unsupported architecture for the Navop portable launcher
#endif

static void fail(const char *message) {
    fprintf(stderr, "navop portable launcher: %s\n", message);
    exit(127);
}

static char *read_self_path(void) {
    size_t capacity = 256;

    for (;;) {
        char *buffer = malloc(capacity);
        if (buffer == NULL) {
            fail("out of memory while resolving the launcher path");
        }

        ssize_t length = readlink("/proc/self/exe", buffer, capacity - 1);
        if (length < 0) {
            free(buffer);
            fail("cannot resolve /proc/self/exe");
        }
        if ((size_t)length < capacity - 1) {
            buffer[length] = '\0';
            return buffer;
        }

        free(buffer);
        capacity *= 2;
        if (capacity > 1024 * 1024) {
            fail("launcher path is unexpectedly long");
        }
    }
}

static char *join_path(const char *base, const char *suffix) {
    size_t base_length = strlen(base);
    size_t suffix_length = strlen(suffix);
    char *path = malloc(base_length + suffix_length + 2);

    if (path == NULL) {
        fail("out of memory while building the runtime path");
    }

    memcpy(path, base, base_length);
    path[base_length] = '/';
    memcpy(path + base_length + 1, suffix, suffix_length + 1);
    return path;
}

static void require_file(const char *path, const char *description) {
    if (access(path, R_OK | X_OK) != 0) {
        fprintf(
            stderr,
            "navop portable launcher: cannot access bundled %s at %s: %s\n",
            description,
            path,
            strerror(errno)
        );
        exit(127);
    }
}

int main(int argc, char **argv) {
    char *self_path = read_self_path();
    char *bin_separator = strrchr(self_path, '/');
    if (bin_separator == NULL) {
        fail("launcher path has no parent directory");
    }
    *bin_separator = '\0';

    char *usr_separator = strrchr(self_path, '/');
    if (usr_separator == NULL) {
        fail("launcher is not installed below usr/bin");
    }
    *usr_separator = '\0';

    char *runtime_root = join_path(self_path, "lib/navop");
    char *library_path = join_path(runtime_root, "lib");
    char *loader_path = join_path(library_path, NAVOP_PORTABLE_LOADER);
    char *binary_path = join_path(runtime_root, "bin/navop.real");
    char *gconv_path = join_path(library_path, "gconv");

    require_file(loader_path, "dynamic loader");
    require_file(binary_path, "application binary");

    unsetenv("LD_AUDIT");
    unsetenv("LD_LIBRARY_PATH");
    unsetenv("LD_PRELOAD");
    unsetenv("LD_PROFILE");
    unsetenv("GLIBC_TUNABLES");
    unsetenv("LOCPATH");
    if (setenv("GCONV_PATH", gconv_path, 1) != 0 ||
        setenv("NAVOP_PORTABLE_ROOT", runtime_root, 1) != 0) {
        fail("cannot configure the bundled runtime environment");
    }

    char **loader_argv = calloc((size_t)argc + 5, sizeof(char *));
    if (loader_argv == NULL) {
        fail("out of memory while preparing application arguments");
    }

    size_t next = 0;
    loader_argv[next++] = loader_path;
    loader_argv[next++] = "--inhibit-cache";
    loader_argv[next++] = "--library-path";
    loader_argv[next++] = library_path;
    loader_argv[next++] = binary_path;
    for (int index = 1; index < argc; index++) {
        loader_argv[next++] = argv[index];
    }
    loader_argv[next] = NULL;

    execv(loader_path, loader_argv);
    fprintf(
        stderr,
        "navop portable launcher: failed to start %s with the bundled loader: %s\n",
        binary_path,
        strerror(errno)
    );
    return 127;
}
