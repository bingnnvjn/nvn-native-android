/*
 * nvn_guest.h — the tiny NVN surface used by the spike guest.
 *
 * These are *spike-local* object layouts, not the real NVN SDK layouts. They
 * exist so the Rust engine has to read guest-owned memory (the `nvn_obj`
 * problem in #19 §3.1) and so a state setter / draw / returns can be exercised
 * without pulling in a full NVN SDK.
 */
#pragma once

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define NVN_MAGIC_DEVICE    0x4e564e4445563031ull /* "NVNDEV01" */
#define NVN_MAGIC_BUILDER   0x4e564e4255493031ull /* "NVNBUI01" */
#define NVN_MAGIC_CMDBUF    0x4e564e434d443031ull /* "NVNCMD01" */
#define NVN_MAGIC_QUEUE     0x4e564e5155453031ull /* "NVNQUE01" */
#define NVN_MAGIC_SAMPLER   0x4e564e534d503031ull /* "NVNSMP01" */
#define NVN_MAGIC_BUFFER    0x4e564e4246463031ull /* "NVNBUF01" */

typedef struct NVNdeviceBuilder { uint64_t magic; uint64_t reserved[7]; } NVNdeviceBuilder;
typedef struct NVNdevice        { uint64_t magic; uint64_t reserved[7]; } NVNdevice;
typedef struct NVNqueue         { uint64_t magic; uint64_t reserved[7]; } NVNqueue;
typedef struct NVNsampler       { uint64_t magic; float lod_bias; uint32_t reserved; uint64_t pad; } NVNsampler;
typedef struct NVNbuffer        { uint64_t magic; uint64_t address; } NVNbuffer;

typedef void *NVNcommandHandle;

typedef enum {
    NVN_OP_CLEAR_COLOR = 1,
    NVN_OP_DRAW_ARRAYS = 2,
    NVN_OP_DEPTH_RANGE = 3
} NVNopKind;

typedef struct NVNop {
    uint32_t kind;
    uint32_t a;
    float    f[4];
    int32_t  i[2];
} NVNop; /* 32 B */

#define NVN_MAX_OPS 16
typedef struct NVNcommandBuffer {
    uint64_t magic;
    uint32_t op_count;
    uint32_t state_flags;
    float    depth_near;
    float    depth_far;
    uint32_t reserved;
    NVNop    ops[NVN_MAX_OPS];
} NVNcommandBuffer;

/* NVNdrawPrimitive value used by the test (NVN_DRAW_TRIANGLES). */
#define NVN_DRAW_TRIANGLES 1

/* ---- resolved NVN entry points (all through nvnBootstrapLoader) ---------- */

typedef void      (*PFN_nvnDeviceBuilderSetDefaults)(NVNdeviceBuilder *);
typedef uint32_t  (*PFN_nvnDeviceInitialize)(NVNdevice *, const NVNdeviceBuilder *);
typedef void     *(*PFN_nvnDeviceGetProcAddress)(const NVNdevice *, const char *);
typedef void      (*PFN_nvnCommandBufferSetDepthRange)(NVNcommandBuffer *, float, float);
typedef void      (*PFN_nvnCommandBufferClearColor)(NVNcommandBuffer *, int, const float *, int);
typedef void      (*PFN_nvnCommandBufferDrawArrays)(NVNcommandBuffer *, int, int, int);
typedef void      (*PFN_nvnQueueSubmitCommands)(NVNqueue *, int, const NVNcommandHandle *);
typedef void      (*PFN_nvnQueueFinish)(NVNqueue *);
typedef float     (*PFN_nvnSamplerGetLodBias)(const NVNsampler *);
typedef uint64_t  (*PFN_nvnBufferGetAddress)(const NVNbuffer *);
typedef void      (*PFN_nvnCommandBufferDispatchCompute)(NVNcommandBuffer *, int, int, int);
typedef uint64_t  (*PFN_nvnDeviceGetCurrentTimestampInNanoseconds)(const NVNdevice *);
typedef uint64_t  (*PFN_nvaSpikeTlsProbe)(void);
typedef uint64_t  (*PFN_nvaSpikeStackProbe)(uint64_t, uint64_t, uint64_t, uint64_t,
                                            uint64_t, uint64_t, uint64_t, uint64_t,
                                            uint64_t, uint64_t);

#ifdef __cplusplus
}
#endif
