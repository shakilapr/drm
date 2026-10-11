/*
 * Compile-fail fixture (I-B1 surface): a Borrow guard is move-only; the copy
 * constructor is deleted, so this MUST NOT compile.
 * REQ: REQ-BIND-013.
 */
#include <drm/drm.hpp>

int main() {
    drm::Borrow<int> original;
    drm::Borrow<int> copy = original; /* ERROR: deleted copy constructor */
    (void)copy;
    return 0;
}
