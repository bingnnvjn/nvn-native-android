/*
 * nva_abi.h — C ABI v1 subset exercised by the #30 ABI spike.
 *
 * This is deliberately NOT the full v1 header. It contains only the pieces the
 * spike needs, with the v1 field order/size kept byte-for-byte where they exist:
 *   - nva_host_services / nva_gfx_desc / nva_gfx_create
 *   - nva_nvn_table / nva_nvn_slot / nva_nvn_resolve
 * The *_spike_* symbols are test hooks, not part of C ABI v1.
 *
 * ABI rules:
 *   - all structs begin with {uint32_t size; uint32_t version;}
 *   - pointers are 64-bit; the guest and host are in one address space
 *   - the table is engine-owned and borrowed until nva_gfx_destroy()
 */
#pragma once

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

#define NVA_ABI_VERSION 1u

typedef uint32_t nva_status;
enum {
    NVA_OK = 0,
    NVA_ERR_INVALID_ARGUMENT = 1,
    NVA_ERR_UNSUPPORTED = 2,
    NVA_ERR_DEVICE_LOST = 3,
    NVA_ERR_STALE = 4,
    NVA_ERR_OUT_OF_MEMORY = 5
};

#define NVA_STRUCT_INIT(T) { .size = (uint32_t)sizeof(T), .version = (uint32_t)(T##_VERSION) }

typedef struct nva_gfx nva_gfx;

/* Forward declaration only: the spike never allocates guest memory. */
typedef struct nva_mem_region {
    uint32_t size, version;
    uint64_t guest_va, size_bytes;
    uint32_t kind, perms, owner, flags;
    uint64_t alloc_handle;
    void    *cpu_ptr;
    uint64_t gpu_handle;
    uint64_t gpu_offset;
    uint64_t alias_of;
    uint32_t coherence, map_state;
    uint64_t region_id, epoch;
} nva_mem_region; /* 104 B, align 8, v1 */

typedef struct nva_host_services {
    uint32_t size, version;
    void *user;
    void (*log)(void *user, uint32_t level, const char *tag, const char *msg);
    nva_status (*mem_query)(void *user, uint64_t va, uint64_t size,
                            nva_mem_region *out, uint64_t *out_off,
                            uint64_t *out_covered);
    void (*on_device_lost)(void *user, uint32_t reason, uint64_t serial);
    nva_status (*shader_lookup)(void *user, const void *key, void *out);
} nva_host_services;
#define NVA_HOST_SERVICES_VERSION 1u

typedef struct nva_gfx_desc {
    uint32_t size, version, flags;
    const nva_host_services *host; /* must be non-NULL in v1 */
} nva_gfx_desc;
#define NVA_GFX_DESC_VERSION 1u

nva_status nva_gfx_create(const nva_gfx_desc *desc, nva_gfx **out);
void       nva_gfx_destroy(nva_gfx *gfx);

/* ---- NVN table (v1 shape) ------------------------------------------------- */

typedef struct nva_nvn_slot {
    uint64_t count;  /* incremented by the guest thunk (stadd / lock incq) */
    uint64_t target; /* host implementation (AArch64: br target) */
} nva_nvn_slot;      /* 16 B */

typedef struct nva_nvn_table {
    uint32_t size, version;
    uint32_t slot_count; /* spike subset, not the full 523 */
    uint32_t entry_size; /* 16 */
    uint64_t slots;      /* const nva_nvn_slot*; engine-owned, not a guest VA */
    uint64_t names;      /* const char *const *; index = slot */
    uint64_t names_hash;
    uint32_t flags, reserved;
} nva_nvn_table;
#define NVA_NVN_TABLE_VERSION 1u

/* table flags */
#define NVA_NVN_TABLE_GATE_TLS 0x1u /* resolve() returns TLS-swapping gate thunks */
#define NVA_NVN_TABLE_LSE      0x2u /* thunks use stadd (ARMv8.1-LSE) */

/* v1 entry point: returns a guest-callable thunk VA, never NULL for a
 * well-formed table; unknown names return a generic zero stub. */
uint64_t nva_nvn_resolve(const nva_nvn_table *table, const char *name);

const nva_nvn_table *nva_gfx_get_nvn_table(nva_gfx *gfx);

/* ---- spike-only resolve extension (candidate C ABI v1 addition) ----------- */

typedef struct nva_nvn_resolve_result {
    uint64_t target;  /* thunk VA */
    uint32_t slot;    /* index, or UINT32_MAX for unknown */
    uint32_t flags;   /* NVA_NVN_RESOLVE_* */
} nva_nvn_resolve_result;

#define NVA_NVN_RESOLVE_KNOWN   0x1u
#define NVA_NVN_RESOLVE_UNKNOWN 0x2u
#define NVA_NVN_RESOLVE_GATE    0x4u

nva_status nva_nvn_resolve_ex(const nva_nvn_table *table, const char *name,
                              nva_nvn_resolve_result *out);

/* ---- NVN table-external entries (implemented by the host/engine) ---------- */

void *nvnBootstrapLoader(const char *name);
void *nvnDeviceGetProcAddress(const void *device, const char *name);

/* ---- spike test hooks (NOT C ABI v1) ------------------------------------- */

#define NVA_SPIKE_VERSION 1u
typedef struct nva_spike_state {
    uint32_t size, version;
    uint32_t ok;                 /* engine initialized */
    uint32_t tls_gate;           /* 1 = table flags include GATE_TLS */
    uint32_t validation_available;
    uint32_t validation_errors;
    uint32_t unsupported_calls;  /* typed zero stub hits */
    uint32_t unknown_resolves;   /* unknown name resolves */
    uint32_t stub_aborts;        /* abort stub hits (normally read post-mortem) */
    uint32_t fence_wait_ok;
    uint32_t last_serial;
    uint32_t pixel_rgba;         /* RGBA8, little-endian */
    uint32_t depth_bits_near;    /* float bits as seen by the engine */
    uint32_t depth_bits_far;
    uint64_t tls_at_last_entry;  /* TPIDR_EL0 observed by the host impl */
    uint64_t host_tls;           /* TPIDR_EL0 captured at nva_gfx_create */
    uint64_t slot_counts[16];    /* per-slot thunk counters */
    char backend[64];
    char device[128];
} nva_spike_state;
#define NVA_SPIKE_STATE_VERSION 1u

typedef struct nva_spike_stack {
    uint64_t sum;
    uint64_t sp_at_entry;
} nva_spike_stack;
#define NVA_SPIKE_STACK_VERSION 1u

nva_status nva_spike_get_state(nva_gfx *gfx, nva_spike_state *out);
nva_status nva_spike_read_pixel(nva_gfx *gfx, uint32_t *out_rgba);
nva_status nva_spike_set_tls_gate(nva_gfx *gfx, uint32_t enable);
nva_status nva_spike_wait(nva_gfx *gfx, uint64_t timeout_ns);

/* Returns TPIDR_EL0 at host entry (0 on non-AArch64). Called through the
 * table, so it also measures whether a TLS gate swapped before the impl. */
uint64_t nvaSpikeTlsProbe(void);
/* Ten integer args: exercises stack arguments in both thunk shapes. */
uint64_t nvaSpikeStackProbe(uint64_t a0, uint64_t a1, uint64_t a2, uint64_t a3,
                            uint64_t a4, uint64_t a5, uint64_t a6, uint64_t a7,
                            uint64_t a8, uint64_t a9);
/* 16-byte struct return (x0/x1), and a 24-byte struct return via x8. */
typedef struct nvaSpikePair { uint64_t a, b; } nvaSpikePair;
nvaSpikePair nvaSpikeReturnPair(void);
typedef struct nvaSpikeTriple { uint64_t a, b, c; } nvaSpikeTriple;
nvaSpikeTriple nvaSpikeReturnTriple(void);

#ifdef __cplusplus
}
#endif
