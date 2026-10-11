/*
 * Compile-fail fixture (I-B2): the binding exposes only declared operations;
 * an undeclared endpoint op must not compile.
 * REQ: REQ-BIND-011 / REQ-BIND-012.
 */
#include <drm/drm.hpp>

int main() {
    /* No such operation is declared by the binding. */
    return drm::publish_undeclared_topic(nullptr, 0, nullptr, 0);
}
