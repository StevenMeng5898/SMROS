#define _GNU_SOURCE

/*
 * ARM NEON tensor multiply demo for SMROS.
 *
 * Batched GEMM (rank-3 tensor contraction):
 *
 *   C[b, m, n] = sum_k A[b, m, k] * B[b, k, n]
 *
 * The AArch64 PIE uses a 4x4 NEON FMLA microkernel. Rebuild the kernel after
 * `make -C src/user_level/apps smros` so host_shared/ is embedded as /shared.
 *
 * On SMROS:
 *   run tensor_neon_mul.elf
 *   run tensor_neon_mul.elf 2 32 32 32 3
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#if defined(__ARM_NEON) || defined(__ARM_NEON__)
#include <arm_neon.h>
#define TENSOR_HAS_NEON 1
#else
#define TENSOR_HAS_NEON 0
#endif

static void die(const char *msg)
{
    perror(msg);
    exit(1);
}

static void *alloc_aligned(size_t bytes)
{
    void *ptr = NULL;
    if (posix_memalign(&ptr, 64, bytes) != 0 || ptr == NULL) {
        die("posix_memalign");
    }
    memset(ptr, 0, bytes);
    return ptr;
}

static float *tensor3(int batch, int dim0, int dim1)
{
    return (float *)alloc_aligned((size_t)batch * (size_t)dim0 * (size_t)dim1 * sizeof(float));
}

static float fill_value(int batch, int row, int col, int tag)
{
    int mixed = batch * 131 + row * 17 + col * 13 + tag;
    return (float)((mixed % 201) - 100) * 0.015625f;
}

static void fill_tensor(float *t, int batch, int dim0, int dim1, int tag)
{
    int b, r, c;
    for (b = 0; b < batch; b++) {
        for (r = 0; r < dim0; r++) {
            for (c = 0; c < dim1; c++) {
                t[((size_t)b * dim0 + r) * dim1 + c] = fill_value(b, r, c, tag);
            }
        }
    }
}

static double now_sec(void)
{
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) {
        die("clock_gettime");
    }
    return (double)ts.tv_sec + (double)ts.tv_nsec * 1e-9;
}

static void gemm_scalar(
    const float *a,
    const float *b,
    float *c,
    int m,
    int n,
    int k)
{
    int row, col, kk;
    for (row = 0; row < m; row++) {
        for (col = 0; col < n; col++) {
            float acc = 0.0f;
            for (kk = 0; kk < k; kk++) {
                acc += a[row * k + kk] * b[kk * n + col];
            }
            c[row * n + col] = acc;
        }
    }
}

static void tensor_mul_scalar(
    const float *a,
    const float *b,
    float *c,
    int batch,
    int m,
    int n,
    int k)
{
    int bi;
    size_t a_stride = (size_t)m * (size_t)k;
    size_t b_stride = (size_t)k * (size_t)n;
    size_t c_stride = (size_t)m * (size_t)n;
    for (bi = 0; bi < batch; bi++) {
        gemm_scalar(a + bi * a_stride, b + bi * b_stride, c + bi * c_stride, m, n, k);
    }
}

#if TENSOR_HAS_NEON
static void gemm_neon_4x4(
    const float *a,
    const float *b,
    float *c,
    int n,
    int k)
{
    int kk;
    float32x4_t c0 = vdupq_n_f32(0.0f);
    float32x4_t c1 = vdupq_n_f32(0.0f);
    float32x4_t c2 = vdupq_n_f32(0.0f);
    float32x4_t c3 = vdupq_n_f32(0.0f);

    for (kk = 0; kk < k; kk++) {
        float32x4_t bvec = vld1q_f32(b + kk * n);
        c0 = vfmaq_n_f32(c0, bvec, a[0 * k + kk]);
        c1 = vfmaq_n_f32(c1, bvec, a[1 * k + kk]);
        c2 = vfmaq_n_f32(c2, bvec, a[2 * k + kk]);
        c3 = vfmaq_n_f32(c3, bvec, a[3 * k + kk]);
    }

    vst1q_f32(c + 0 * n, c0);
    vst1q_f32(c + 1 * n, c1);
    vst1q_f32(c + 2 * n, c2);
    vst1q_f32(c + 3 * n, c3);
}

static void gemm_neon(
    const float *a,
    const float *b,
    float *c,
    int m,
    int n,
    int k)
{
    int row, col;

    for (row = 0; row + 4 <= m; row += 4) {
        for (col = 0; col + 4 <= n; col += 4) {
            gemm_neon_4x4(a + row * k, b + col, c + row * n + col, n, k);
        }
        for (; col < n; col++) {
            int r, kk;
            for (r = 0; r < 4; r++) {
                float acc = 0.0f;
                for (kk = 0; kk < k; kk++) {
                    acc += a[(row + r) * k + kk] * b[kk * n + col];
                }
                c[(row + r) * n + col] = acc;
            }
        }
    }

    for (; row < m; row++) {
        for (col = 0; col < n; col++) {
            int kk;
            float acc = 0.0f;
            for (kk = 0; kk < k; kk++) {
                acc += a[row * k + kk] * b[kk * n + col];
            }
            c[row * n + col] = acc;
        }
    }
}

static void tensor_mul_neon(
    const float *a,
    const float *b,
    float *c,
    int batch,
    int m,
    int n,
    int k)
{
    int bi;
    size_t a_stride = (size_t)m * (size_t)k;
    size_t b_stride = (size_t)k * (size_t)n;
    size_t c_stride = (size_t)m * (size_t)n;
    for (bi = 0; bi < batch; bi++) {
        gemm_neon(a + bi * a_stride, b + bi * b_stride, c + bi * c_stride, m, n, k);
    }
}
#endif

static float max_abs_err(const float *x, const float *y, size_t count)
{
    size_t i;
    float worst = 0.0f;
    for (i = 0; i < count; i++) {
        float diff = x[i] - y[i];
        if (diff < 0.0f) {
            diff = -diff;
        }
        if (diff > worst) {
            worst = diff;
        }
    }
    return worst;
}

static double bench(
    void (*fn)(const float *, const float *, float *, int, int, int, int),
    const float *a,
    const float *b,
    float *c,
    int batch,
    int m,
    int n,
    int k,
    int repeats,
    size_t bytes)
{
    int r;
    double t0, t1;
    fn(a, b, c, batch, m, n, k);
    t0 = now_sec();
    for (r = 0; r < repeats; r++) {
        fn(a, b, c, batch, m, n, k);
    }
    t1 = now_sec();
    (void)bytes;
    return (t1 - t0) / (double)repeats;
}

static void usage(const char *argv0)
{
    fprintf(stderr,
            "usage: %s [batch] [M] [N] [K] [repeats]\n"
            "  C[b,m,n] = sum_k A[b,m,k] * B[b,k,n]\n"
            "  defaults: 2 32 32 32 3 (SMROS-friendly)\n",
            argv0);
}

int main(int argc, char **argv)
{
    int batch = 2;
    int m = 32;
    int n = 32;
    int k = 32;
    int repeats = 3;
    float *a;
    float *b;
    float *c_scalar;
    float *c_fast;
    size_t c_count;
    double scalar_s;
    double flops;
    float err;

    if (argc == 2 && (strcmp(argv[1], "-h") == 0 || strcmp(argv[1], "--help") == 0)) {
        usage(argv[0]);
        return 0;
    }
    if (argc >= 2) {
        batch = atoi(argv[1]);
    }
    if (argc >= 3) {
        m = atoi(argv[2]);
    }
    if (argc >= 4) {
        n = atoi(argv[3]);
    }
    if (argc >= 5) {
        k = atoi(argv[4]);
    }
    if (argc >= 6) {
        repeats = atoi(argv[5]);
    }
    if (batch <= 0 || m <= 0 || n <= 0 || k <= 0 || repeats <= 0) {
        usage(argv[0]);
        return 2;
    }

    a = tensor3(batch, m, k);
    b = tensor3(batch, k, n);
    c_scalar = tensor3(batch, m, n);
    c_fast = tensor3(batch, m, n);
    fill_tensor(a, batch, m, k, 1);
    fill_tensor(b, batch, k, n, 2);
    c_count = (size_t)batch * (size_t)m * (size_t)n;
    flops = 2.0 * (double)batch * (double)m * (double)n * (double)k;

    printf("tensor mul  C[b,m,n] = sum_k A[b,m,k] * B[b,k,n]\n");
    printf("shape batch=%d M=%d N=%d K=%d repeats=%d neon=%s\n",
           batch,
           m,
           n,
           k,
           repeats,
           TENSOR_HAS_NEON ? "yes" : "no (scalar fallback)");

    scalar_s = bench(tensor_mul_scalar, a, b, c_scalar, batch, m, n, k, repeats, c_count * sizeof(float));
    printf("scalar  %.3f ms  %.2f GFLOP/s\n", scalar_s * 1e3, flops / scalar_s * 1e-9);

#if TENSOR_HAS_NEON
    {
        int pass;
        double neon_s = bench(
            tensor_mul_neon, a, b, c_fast, batch, m, n, k, repeats, c_count * sizeof(float));
        err = max_abs_err(c_scalar, c_fast, c_count);
        pass = err < 1e-3f;
        printf("neon    %.3f ms  %.2f GFLOP/s  speedup=%.2fx\n",
               neon_s * 1e3,
               flops / neon_s * 1e-9,
               scalar_s / neon_s);
        printf("max_abs_err=%.6g  %s\n", err, pass ? "PASS" : "FAIL");
        free(a);
        free(b);
        free(c_scalar);
        free(c_fast);
        return pass ? 0 : 1;
    }
#else
    memcpy(c_fast, c_scalar, c_count * sizeof(float));
    err = max_abs_err(c_scalar, c_fast, c_count);
    printf("rebuild with aarch64-linux-gnu-gcc -march=armv8-a to enable NEON\n");
    printf("max_abs_err=%.6g  %s\n", err, "PASS (scalar only)");
    free(a);
    free(b);
    free(c_scalar);
    free(c_fast);
    return 0;
#endif
}
