/*
 * abi_layout_check.c — C ABI v1 布局守卫（只编译，不链接）
 *
 * 用 host 编译器（x86_64/aarch64 均可）和 NDK clang 各跑一次：
 *   cc -std=c11 -Wall -Wextra -c include/abi_layout_check.c -o /tmp/abi.o
 *   $NDK/toolchains/llvm/prebuilt/<host>/bin/aarch64-linux-android26-clang \
 *      -std=c11 -Wall -Wextra -c include/abi_layout_check.c -o /tmp/abi-arm64.o
 *
 * 任一 _Static_assert 失败即 ABI 漂移；按 CODING_STANDARDS.md「ABI change」
 * 同步 src/lib.rs、nva_abi.h、nvn-table.json、guest.c。
 */
#include "nva_abi.h"

#include <stddef.h>

#if defined(__cplusplus)
#define NVA_SA(c, m) static_assert(c, m)
#else
#define NVA_SA(c, m) _Static_assert(c, m)
#endif

/* ---- nva_mem_region: 104 B ---- */
NVA_SA(sizeof(nva_mem_region) == 104, "nva_mem_region size");
NVA_SA(offsetof(nva_mem_region, guest_va) == 8, "guest_va");
NVA_SA(offsetof(nva_mem_region, size_bytes) == 16, "size_bytes");
NVA_SA(offsetof(nva_mem_region, kind) == 24, "kind");
NVA_SA(offsetof(nva_mem_region, alloc_handle) == 40, "alloc_handle");
NVA_SA(offsetof(nva_mem_region, cpu_ptr) == 48, "cpu_ptr");
NVA_SA(offsetof(nva_mem_region, gpu_handle) == 56, "gpu_handle");
NVA_SA(offsetof(nva_mem_region, gpu_offset) == 64, "gpu_offset");
NVA_SA(offsetof(nva_mem_region, alias_of) == 72, "alias_of");
NVA_SA(offsetof(nva_mem_region, coherence) == 80, "coherence");
NVA_SA(offsetof(nva_mem_region, region_id) == 88, "region_id");
NVA_SA(offsetof(nva_mem_region, epoch) == 96, "epoch");

/* ---- nva_fence: 16 B ---- */
NVA_SA(sizeof(nva_fence) == 16, "nva_fence size");
NVA_SA(offsetof(nva_fence, domain) == 8, "fence domain");
NVA_SA(offsetof(nva_fence, gen) == 12, "fence gen");

/* ---- nva_host_services: 48 B，fatal 在偏移 40 ---- */
NVA_SA(sizeof(nva_host_services) == 48, "nva_host_services size");
NVA_SA(offsetof(nva_host_services, user) == 8, "services user");
NVA_SA(offsetof(nva_host_services, log) == 16, "services log");
NVA_SA(offsetof(nva_host_services, mem_query) == 24, "services mem_query");
NVA_SA(offsetof(nva_host_services, on_device_lost) == 32, "services on_device_lost");
NVA_SA(offsetof(nva_host_services, fatal) == 40, "services fatal");

/* ---- nva_shader_cache: 32 B ---- */
NVA_SA(sizeof(nva_shader_cache) == 32, "nva_shader_cache size");
NVA_SA(offsetof(nva_shader_cache, data) == 16, "shader data");
NVA_SA(offsetof(nva_shader_cache, data_size) == 24, "shader data_size");

/* ---- nva_gfx_desc: 32 B ---- */
NVA_SA(sizeof(nva_gfx_desc) == 32, "nva_gfx_desc size");
NVA_SA(offsetof(nva_gfx_desc, host) == 16, "desc host");
NVA_SA(offsetof(nva_gfx_desc, shader_cache) == 24, "desc shader_cache");

/* ---- nva_gfx_caps: 56 B ---- */
NVA_SA(sizeof(nva_gfx_caps) == 56, "nva_gfx_caps size");
NVA_SA(offsetof(nva_gfx_caps, max_memory_allocation_size) == 32, "caps alloc size");
NVA_SA(offsetof(nva_gfx_caps, min_imported_host_pointer_alignment) == 48, "caps min align");

/* ---- nva_stats: 56 B ---- */
NVA_SA(sizeof(nva_stats) == 56, "nva_stats size");
NVA_SA(offsetof(nva_stats, frames) == 32, "stats frames");

/* ---- NVN ---- */
NVA_SA(sizeof(nva_nvn_slot) == 16, "nvn slot size");
NVA_SA(offsetof(nva_nvn_slot, target) == 8, "nvn slot target");

NVA_SA(sizeof(nva_nvn_slot_meta) == 20, "slot_meta size");
NVA_SA(offsetof(nva_nvn_slot_meta, ret_class) == 4, "meta ret_class");
NVA_SA(offsetof(nva_nvn_slot_meta, stack_bytes) == 10, "meta stack_bytes");
NVA_SA(offsetof(nva_nvn_slot_meta, stub_class) == 12, "meta stub_class");
NVA_SA(offsetof(nva_nvn_slot_meta, flags) == 16, "meta flags");

NVA_SA(sizeof(nva_nvn_table) == 56, "nvn table size");
NVA_SA(offsetof(nva_nvn_table, slot_count) == 8, "table slot_count");
NVA_SA(offsetof(nva_nvn_table, slots) == 16, "table slots");
NVA_SA(offsetof(nva_nvn_table, names) == 24, "table names");
NVA_SA(offsetof(nva_nvn_table, names_hash) == 32, "table names_hash");
NVA_SA(offsetof(nva_nvn_table, flags) == 40, "table flags");
NVA_SA(offsetof(nva_nvn_table, meta) == 48, "table meta");

NVA_SA(sizeof(nva_nvn_resolve_result) == 16, "resolve_result size");
NVA_SA(offsetof(nva_nvn_resolve_result, slot) == 8, "resolve slot");
NVA_SA(offsetof(nva_nvn_resolve_result, flags) == 12, "resolve flags");

/* ---- host ---- */
NVA_SA(sizeof(nva_thread_desc) == 48, "thread_desc size");
NVA_SA(offsetof(nva_thread_desc, stack_size) == 16, "thread stack_size");
NVA_SA(offsetof(nva_thread_desc, entry) == 32, "thread entry");

NVA_SA(sizeof(nva_host_desc) == 32, "host_desc size");
NVA_SA(offsetof(nva_host_desc, data_dir) == 16, "host data_dir");
NVA_SA(offsetof(nva_host_desc, platform) == 24, "host platform");

/* ---- 平台壳 ---- */
NVA_SA(sizeof(nva_audio_config) == 24, "audio_config size");
NVA_SA(sizeof(nva_platform) == 80, "nva_platform size");
NVA_SA(offsetof(nva_platform, audio_open) == 16, "platform audio_open");
NVA_SA(offsetof(nva_platform, log) == 72, "platform log");

NVA_SA(sizeof(nva_input_event) == 32, "input_event size");
NVA_SA(offsetof(nva_input_event, timestamp_ns) == 24, "input timestamp");

NVA_SA(sizeof(nva_app_callbacks) == 48, "app_callbacks size");
NVA_SA(offsetof(nva_app_callbacks, on_fatal) == 40, "callbacks on_fatal");

NVA_SA(sizeof(nva_app_desc) == 40, "app_desc size");
NVA_SA(offsetof(nva_app_desc, callbacks) == 24, "app callbacks");
NVA_SA(offsetof(nva_app_desc, platform) == 32, "app platform");

/* ---- SVC ---- */
NVA_SA(sizeof(nva_svc_frame) == 96, "svc_frame size");
NVA_SA(offsetof(nva_svc_frame, x) == 8, "svc x");
NVA_SA(offsetof(nva_svc_frame, tpidr_el0) == 80, "svc tpidr");
NVA_SA(offsetof(nva_svc_frame, gate) == 88, "svc gate");
NVA_SA(offsetof(nva_svc_frame, flags) == 92, "svc flags");

/* 版本宏必须与文档一致 */
NVA_SA(NVA_ABI_VERSION == 1u, "abi version");
NVA_SA(NVA_HOST_SERVICES_VERSION == 2u, "services version");
NVA_SA(NVA_GFX_DESC_VERSION == 2u, "gfx desc version");
NVA_SA(NVA_NVN_TABLE_VERSION == 2u, "nvn table version");
NVA_SA(NVA_ERR_TIMEOUT == 8, "timeout code");

int nva_abi_layout_check_translation_unit; /* 非空 TU */
