/*
 * Compile-fail fixture (I-B2): the guard never yields a raw pointer into the
 * shared slot, so a `.data()` escape MUST NOT compile.
 * REQ: REQ-BIND-011.
 */
#include <drm/drm.hpp>

int main() {
    drm::Borrow<int> borrow;
    const void *escape = borrow.data(); /* ERROR: no such member (I-B2) */
    (void)escape;
    return 0;
}
