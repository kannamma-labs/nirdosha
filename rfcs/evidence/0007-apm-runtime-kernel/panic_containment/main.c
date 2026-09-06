/* Zero Rust runtime except `libnirdosha_runtime_kernels.a`, linked
 * exactly the way `codegen.rs::build()` links a real compiled `.nir`
 * binary -- a bare `clang <this.c> <runtime.a> -lm -o out` invocation,
 * no `rustc`/`cargo` involvement in producing the final executable at
 * all. This is the real, direct test of whether
 * `kernel::thread_pool`'s `catch_unwind`-based panic containment
 * (crates/runtime-kernels/src/kernel/thread_pool.rs) actually survives
 * in the environment that matters -- not `cargo test`'s own dev-profile
 * unit test runner, which already has a full Rust runtime of its own
 * and would mask exactly the failure mode under test.
 *
 * Exit code 1 = containment held (the pool survived the panic and ran
 * a job after it). Exit code 0 = the self-test function itself
 * reported failure. No output/a crash instead = containment did NOT
 * survive -- the process aborted before `nir_kernel_self_test_panic_
 * containment` could even return, which is itself the answer.
 */
#include <stdio.h>

extern int nir_kernel_self_test_panic_containment(void);

int main(void) {
    int result = nir_kernel_self_test_panic_containment();
    printf("nir_kernel_self_test_panic_containment() = %d\n", result);
    return result == 1 ? 0 : 1;
}
