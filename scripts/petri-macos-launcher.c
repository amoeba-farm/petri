// A Mach-O entrypoint keeps the app signature in ordinary package bytes.
// Script signatures can live in extended attributes and be lost in an update:
// https://developer.apple.com/library/archive/technotes/tn2206/_index.html
#include <limits.h>
#include <mach-o/dyld.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int main(void) {
    char executable[PATH_MAX];
    char resolved[PATH_MAX];
    char command[PATH_MAX];
    uint32_t size = sizeof executable;
    if (_NSGetExecutablePath(executable, &size) != 0 ||
        realpath(executable, resolved) == NULL) {
        return 1;
    }
    // Contents/MacOS/Petri -> Contents/Resources/petri.command.
    for (int i = 0; i < 2; ++i) {
        char *separator = strrchr(resolved, '/');
        if (separator == NULL) return 1;
        *separator = '\0';
    }
    int length = snprintf(command, sizeof command, "%s/Resources/petri.command", resolved);
    if (length < 0 || (size_t)length >= sizeof command) return 1;
    execl("/usr/bin/open", "open", "-a", "Terminal", command, (char *)NULL);
    perror("Could not open Petri in Terminal");
    return 1;
}
