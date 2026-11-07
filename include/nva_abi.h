/*
 * nva_abi.h — C ABI v1（2026-10-30 冻结）
 *
 * 三条接缝：
 *   壳(JNI C glue) → runtime   : nva_app_*
 *   runtime → 壳               : nva_platform（壳实现）
 *   runtime → host             : nva_host_* / nva_mem_query / nva_host_svc_dispatch
 *   host → engine              : nva_gfx_*（控制面）、nva_host_services（engine→host）
 *   loader → engine            : nva_nvn_*（Rust 调用，布局冻结）
 *   guest → engine             : thunk（AAPCS64 机器 ABI，不是 C ABI）
 *
 * 规则：
 *   - 全 extern "C"，固定宽度；结构体首两字段 size/version。
 *   - 只尾部追加新字段并升该结构 version；消费方按 version 分支。
 *   - Rust 侧 panic=abort；不允许 unwind 穿过 FFI。
 *   - host 编译 +reserve-x18；thunk/gate/impl 不读写 x18；NVNboolean 是 8-bit。
 *
 * 布局由 include/abi_layout_check.c 的 _Static_assert 守卫。
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
    NVA_OK                    = 0,
    NVA_ERR_INVALID_ARGUMENT  = 1,
    NVA_ERR_UNSUPPORTED       = 2,
    NVA_ERR_DEVICE_LOST       = 3,
    NVA_ERR_STALE             = 4,
    NVA_ERR_OUT_OF_MEMORY     = 5,
    NVA_ERR_OUT_OF_DATE       = 6,
    NVA_ERR_SURFACE_LOST      = 7,
    NVA_ERR_TIMEOUT           = 8
};

/* 两参数：NVA_STRUCT_INIT(nva_gfx_desc, NVA_GFX_DESC_VERSION) */
#define NVA_STRUCT_INIT(T, V) { .size = (uint32_t)sizeof(T), .version = (uint32_t)(V) }

typedef struct nva_gfx  nva_gfx;
typedef struct nva_host nva_host;
typedef struct nva_app  nva_app;
typedef struct nva_platform nva_platform;
typedef struct nva_svc_frame nva_svc_frame;

/* ------------------------------------------------------------------ 内存 */

typedef struct nva_mem_region {          /* 104 B, align 8 */
    uint32_t size, version;
    uint64_t guest_va, size_bytes;
    uint32_t kind, perms, owner, flags;
    uint64_t alloc_handle;               /* backing 身份 */
    void    *cpu_ptr;                    /* 只在 map_state & CPU 期间有效 */
    uint64_t gpu_handle;                 /* engine 私有；COPY-only 为 0 */
    uint64_t gpu_offset;                 /* 本 region 在 backing 内偏移 */
    uint64_t alias_of;                   /* 规范映射的 region_id；0 = 自己 */
    uint32_t coherence, map_state;
    uint64_t region_id, epoch;
} nva_mem_region;
#define NVA_MEM_REGION_VERSION 1u

enum {
    NVA_MEM_KIND_UNKNOWN = 0, NVA_MEM_KIND_CODE = 1, NVA_MEM_KIND_DATA = 2,
    NVA_MEM_KIND_HEAP = 3, NVA_MEM_KIND_STACK = 4, NVA_MEM_KIND_TLS = 5,
    NVA_MEM_KIND_GATE = 6, NVA_MEM_KIND_NVN_TABLE = 7, NVA_MEM_KIND_SHADER = 8,
    NVA_MEM_KIND_ROMFS = 9, NVA_MEM_KIND_GPU = 10, NVA_MEM_KIND_NVMAP = 11
};
enum { NVA_MEM_OWNER_NONE = 0, NVA_MEM_OWNER_LOADER = 1,
       NVA_MEM_OWNER_HOST = 2, NVA_MEM_OWNER_ENGINE = 3, NVA_MEM_OWNER_PLATFORM = 4 };
enum { NVA_MEM_PERM_R = 1u, NVA_MEM_PERM_W = 2u, NVA_MEM_PERM_X = 4u };
enum { NVA_MEM_COHERENCE_UNTRACKED = 0, NVA_MEM_COHERENCE_HOST_FLUSHED = 1,
       NVA_MEM_COHERENCE_IMMUTABLE = 2 };
enum { NVA_MEM_MAP_NONE = 0, NVA_MEM_MAP_CPU = 1, NVA_MEM_MAP_GPU = 2, NVA_MEM_MAP_CPU_GPU = 3 };

typedef struct nva_fence {               /* 16 B */
    uint64_t serial;
    uint32_t domain, gen;
} nva_fence;
#define NVA_FENCE_VERSION 1u

/* ------------------------------------------------------- host services */

typedef struct nva_host_services {       /* 48 B */
    uint32_t size, version;              /* NVA_HOST_SERVICES_VERSION = 2 */
    void *user;
    void (*log)(void *user, uint32_t level, const char *tag, const char *msg);
    nva_status (*mem_query)(void *user, uint64_t va, uint64_t size,
                            nva_mem_region *out, uint64_t *out_off,
                            uint64_t *out_covered);
    void (*on_device_lost)(void *user, uint32_t reason, uint64_t serial);
    /* engine 只通知，不自行 abort()/exit()；host 决定墓碑/停 guest/UI 报错 */
    void (*fatal)(void *user, uint32_t reason, uint32_t slot, const char *name);
} nva_host_services;
#define NVA_HOST_SERVICES_VERSION 2u   /* v1(spike) 的偏移 40 是 shader_lookup，已删 */

/* --------------------------------------------------------- shader cache */

typedef struct nva_shader_cache {        /* 32 B；runtime mmap，engine 借用 */
    uint32_t size, version;
    uint32_t format_version, reserved;
    const void *data;
    uint64_t data_size;
} nva_shader_cache;
#define NVA_SHADER_CACHE_VERSION 1u

/* ---------------------------------------------------------------- gfx */

typedef struct nva_gfx_desc {            /* 32 B */
    uint32_t size, version, flags, reserved;
    const nva_host_services *host;       /* 必须非 NULL；生命周期 = gfx */
    const nva_shader_cache  *shader_cache; /* 可 NULL；生命周期 = gfx */
} nva_gfx_desc;
#define NVA_GFX_DESC_VERSION 2u

typedef struct nva_gfx_caps {            /* 56 B */
    uint32_t size, version;
    uint32_t max_color_attachments, max_viewports;
    uint32_t max_push_constants, max_msaa_samples;
    uint32_t flags, reserved;
    uint64_t max_memory_allocation_size;
    uint32_t max_memory_allocation_count, reserved2;
    uint64_t min_imported_host_pointer_alignment;
} nva_gfx_caps;
#define NVA_GFX_CAPS_VERSION 1u
#define NVA_GFX_CAP_VULKAN_11        0x1u
#define NVA_GFX_CAP_CONDITIONAL_QUERY 0x2u

typedef struct nva_stats {               /* 56 B */
    uint32_t size, version;
    uint32_t unsupported_calls, unknown_resolves, stub_aborts, sticky_error;
    uint32_t last_serial, flags;
    uint64_t frames, draw_calls, submits;
} nva_stats;
#define NVA_STATS_VERSION 1u

nva_status nva_gfx_create(const nva_gfx_desc *desc, nva_gfx **out);
void       nva_gfx_destroy(nva_gfx *gfx);
nva_status nva_gfx_query_caps(nva_gfx *gfx, nva_gfx_caps *out);
nva_status nva_gfx_wait_idle(nva_gfx *gfx);
nva_status nva_gfx_register_host_memory(nva_gfx *gfx, const nva_mem_region *region, uint32_t flags);
nva_status nva_gfx_unregister_host_memory(nva_gfx *gfx, uint64_t alloc_handle);
nva_status nva_gfx_alloc_gpu_backing(nva_gfx *gfx, uint64_t size, uint64_t align,
                                     uint32_t flags, nva_mem_region *out);
nva_status nva_gfx_free_gpu_backing(nva_gfx *gfx, uint64_t alloc_handle);
nva_status nva_gfx_notify_dirty(nva_gfx *gfx, uint64_t va, uint64_t size, uint64_t epoch);
nva_status nva_gfx_wait_fence(nva_gfx *gfx, const nva_fence *fence, uint64_t timeout_ns);
nva_status nva_gfx_get_stats(nva_gfx *gfx, nva_stats *out);

/* ---------------------------------------------------------------- NVN */

typedef struct nva_nvn_slot {            /* 16 B；engine-owned */
    uint64_t count;                      /* guest thunk 自增 */
    uint64_t target;                     /* host 实现地址 */
} nva_nvn_slot;

#define NVA_NVN_RET_VOID       0u
#define NVA_NVN_RET_BOOL8      1u
#define NVA_NVN_RET_I32        2u
#define NVA_NVN_RET_U64        3u
#define NVA_NVN_RET_F32        4u
#define NVA_NVN_RET_PTR        5u
#define NVA_NVN_RET_STRUCT16   6u
#define NVA_NVN_RET_STRUCT_GT16 7u
#define NVA_NVN_RET_UNKNOWN    0xFFu

#define NVA_NVN_STUB_IMPL      0u
#define NVA_NVN_STUB_TYPED_ZERO 1u
#define NVA_NVN_STUB_ABORT     2u
#define NVA_NVN_STUB_UNKNOWN   3u

typedef struct nva_nvn_slot_meta {       /* 20 B；由 table.version 版本化 */
    uint32_t slot;
    uint16_t ret_class, arg_gpr, arg_fpr, stack_bytes;
    uint32_t stub_class, flags;
} nva_nvn_slot_meta;

typedef struct nva_nvn_table {           /* 56 B；engine-owned，借用到 gfx_destroy */
    uint32_t size, version;              /* NVA_NVN_TABLE_VERSION = 2 */
    uint32_t slot_count;                 /* 523 */
    uint32_t entry_size;                 /* 16 */
    uint64_t slots;                      /* const nva_nvn_slot*，不是 guest VA */
    uint64_t names;                      /* const char *const* */
    uint64_t names_hash;
    uint32_t flags, reserved;
    uint64_t meta;                       /* const nva_nvn_slot_meta*；v1 追加 */
} nva_nvn_table;
#define NVA_NVN_TABLE_VERSION 2u
#define NVA_NVN_TABLE_DIRECT   0x0u      /* 重写器已把 guest TLS 移出 TPIDR_EL0 */
#define NVA_NVN_TABLE_GATE_TLS 0x1u      /* v1 默认：gate 交换 TPIDR_EL0 */
#define NVA_NVN_TABLE_LSE      0x2u      /* thunk 用 stadd */

uint64_t nva_nvn_resolve(const nva_nvn_table *table, const char *name);

typedef struct nva_nvn_resolve_result {  /* 16 B */
    uint64_t target;
    uint32_t slot, flags;
} nva_nvn_resolve_result;
#define NVA_NVN_RESOLVE_KNOWN   0x1u
#define NVA_NVN_RESOLVE_UNKNOWN 0x2u
#define NVA_NVN_RESOLVE_GATE    0x4u

nva_status nva_nvn_resolve_ex(const nva_nvn_table *table, const char *name,
                              nva_nvn_resolve_result *out);
uint64_t   nva_nvn_bootstrap(nva_gfx *gfx, const char *name);
nva_status nva_nvn_set_thread_tls(nva_gfx *gfx, uint64_t host_tls);

const nva_nvn_table *nva_gfx_get_nvn_table(nva_gfx *gfx);

/* guest 可见符号（由 engine 提供；签名是机器 ABI，不是给 C 调用的稳定 API） */
void *nvnBootstrapLoader(const char *name);
void *nvnDeviceGetProcAddress(const void *device, const char *name);

/* --------------------------------------------------------------- host */

typedef struct nva_thread_desc {         /* 48 B */
    uint32_t size, version;
    uint32_t priority, flags;
    uint64_t stack_size, tls_size;
    uint64_t entry, arg;
} nva_thread_desc;
#define NVA_THREAD_DESC_VERSION 1u

typedef struct nva_host_desc {           /* 32 B */
    uint32_t size, version, flags, reserved;
    const char *data_dir;                /* 私有目录，POSIX 路径 */
    const nva_platform *platform;        /* 壳实现；可 NULL（无音频时） */
} nva_host_desc;
#define NVA_HOST_DESC_VERSION 1u

nva_status nva_host_create(const nva_host_desc *desc, nva_host **out);
void       nva_host_destroy(nva_host *host);
nva_status nva_host_start(nva_host *host);
void       nva_host_stop(nva_host *host);
nva_status nva_host_set_gate_table(nva_host *host, const void *gates,
                                   uint64_t size, uint64_t hash);
nva_status nva_host_svc_dispatch(nva_host *host, nva_svc_frame *frame);

nva_status nva_host_thread_create(nva_host *host, const nva_thread_desc *desc,
                                  uint64_t *out_tid);
nva_status nva_host_thread_join(nva_host *host, uint64_t tid);
nva_status nva_host_thread_detach(nva_host *host, uint64_t tid);

nva_status nva_mem_query(nva_host *host, uint64_t va, uint64_t size,
                         nva_mem_region *out, uint64_t *out_off,
                         uint64_t *out_covered);

/* --------------------------------------------------------------- 平台壳 */

typedef struct nva_audio_config {        /* 24 B */
    uint32_t size, version;
    uint32_t sample_rate, channels;
    uint32_t format, flags;
} nva_audio_config;
#define NVA_AUDIO_CONFIG_VERSION 1u
#define NVA_AUDIO_FORMAT_I16 1u

struct nva_platform {                    /* 80 B；壳实现，runtime 借用至 app destroy */
    uint32_t size, version;
    void *user;
    nva_status (*audio_open)(void *user, const nva_audio_config *cfg, uint64_t *out_stream);
    nva_status (*audio_write)(void *user, uint64_t stream, const void *data, uint32_t bytes);
    nva_status (*audio_flush)(void *user, uint64_t stream);
    nva_status (*audio_close)(void *user, uint64_t stream);
    nva_status (*audio_set_volume)(void *user, uint64_t stream, float volume);
    nva_status (*rumble)(void *user, uint32_t low, uint32_t high, uint32_t duration_ms);
    uint64_t   (*clock_now_ns)(void *user, uint32_t clock_id);
    void       (*log)(void *user, uint32_t level, const char *tag, const char *msg);
};
#define NVA_PLATFORM_VERSION 1u

typedef struct nva_input_event {         /* 32 B */
    uint32_t size, version;
    uint32_t type, code;
    int32_t  value;
    uint32_t flags;
    uint64_t timestamp_ns;
} nva_input_event;
#define NVA_INPUT_EVENT_VERSION 1u
enum { NVA_INPUT_KEY = 1, NVA_INPUT_AXIS = 2, NVA_INPUT_TOUCH = 3,
       NVA_INPUT_GYRO = 4, NVA_INPUT_RUMBLE = 5 };

typedef struct nva_app_callbacks {       /* 48 B */
    uint32_t size, version;
    void *user;
    void (*on_surface_ready)(void *user, uint32_t generation, uint32_t width, uint32_t height);
    void (*on_surface_lost)(void *user, uint32_t generation);
    void (*on_device_lost)(void *user, uint32_t reason, uint64_t serial);
    void (*on_fatal)(void *user, uint32_t reason, const char *message);
} nva_app_callbacks;
#define NVA_APP_CALLBACKS_VERSION 1u

typedef struct nva_app_desc {            /* 40 B */
    uint32_t size, version, flags, reserved;
    const char *data_dir;
    const nva_app_callbacks *callbacks;
    const nva_platform *platform;
} nva_app_desc;
#define NVA_APP_DESC_VERSION 1u

nva_status nva_app_create(const nva_app_desc *desc, nva_app **out);
void       nva_app_destroy(nva_app *app);
nva_status nva_app_start(nva_app *app);
nva_status nva_app_pause(nva_app *app);
nva_status nva_app_resume(nva_app *app);
nva_status nva_app_stop(nva_app *app);
nva_status nva_app_attach_window(nva_app *app, void *native_window,
                                 uint32_t width, uint32_t height, uint32_t generation);
nva_status nva_app_detach_window(nva_app *app, uint32_t generation);
nva_status nva_app_push_input(nva_app *app, const nva_input_event *event);
nva_status nva_app_set_rumble(nva_app *app, uint32_t low, uint32_t high, uint32_t duration_ms);
nva_status nva_app_get_stats(nva_app *app, nva_stats *out);
nva_status nva_app_trim_memory(nva_app *app);

/* ---------------------------------------------------------------- SVC */

/* 96 B；gate 汇编填写；host 只读；guest 线程栈上，host 要留就复制 */
struct nva_svc_frame {
    uint32_t size, version;
    uint64_t x[9];                       /* x0..x8；x0 返回、x1..x7 参数、x8 间接结果 */
    uint64_t tpidr_el0;                  /* 进入 host 前的 guest TLS，只读 */
    uint32_t gate;
    uint32_t flags;
};
#define NVA_SVC_FRAME_VERSION 1u

#ifdef __cplusplus
}
#endif
