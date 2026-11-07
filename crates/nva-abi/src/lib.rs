//! Rust mirror of the product C ABI header `include/nva_abi.h` (#41).
//!
//! Contract (#37 D3): the C header is the single source of truth and this crate
//! is its mirror. Every type, constant and field below carries the same name and
//! the same layout as the C side; the `const` assertion block near the bottom
//! pins exactly the sizes and offsets that `include/abi_layout_check.c` pins, so
//! a mismatch on either side fails **compilation**, not a runtime check.
//!
//! Precondition: 64-bit LP64 targets only (PC x86_64, Android aarch64). `void *`,
//! `char *` and function pointers are 8 bytes there, which is what both the C and
//! the Rust assertions assume.
//!
//! Deliberately not mirrored: function prototypes, the opaque `nva_gfx` /
//! `nva_host` / `nva_app` handles (they only appear in prototypes), and the
//! `NVA_STRUCT_INIT` convenience macro. This crate has no runtime logic.
#![forbid(unsafe_code)]
#![allow(non_camel_case_types)]

use core::ffi::{c_char, c_void};

// ------------------------------------------------------------------ 内存

/// `uint32_t` status code; the values are the `NVA_OK` / `NVA_ERR_*` constants.
pub type nva_status = u32;

pub const NVA_ABI_VERSION: u32 = 1;

pub const NVA_OK: nva_status = 0;
pub const NVA_ERR_INVALID_ARGUMENT: nva_status = 1;
pub const NVA_ERR_UNSUPPORTED: nva_status = 2;
pub const NVA_ERR_DEVICE_LOST: nva_status = 3;
pub const NVA_ERR_STALE: nva_status = 4;
pub const NVA_ERR_OUT_OF_MEMORY: nva_status = 5;
pub const NVA_ERR_OUT_OF_DATE: nva_status = 6;
pub const NVA_ERR_SURFACE_LOST: nva_status = 7;
pub const NVA_ERR_TIMEOUT: nva_status = 8;

/// 104 B, align 8.
#[repr(C)]
pub struct nva_mem_region {
    pub size: u32,
    pub version: u32,
    pub guest_va: u64,
    pub size_bytes: u64,
    pub kind: u32,
    pub perms: u32,
    pub owner: u32,
    pub flags: u32,
    pub alloc_handle: u64,
    /// Only valid while `map_state & CPU`.
    pub cpu_ptr: *mut c_void,
    /// Engine-private backing handle; 0 for COPY-only regions.
    pub gpu_handle: u64,
    /// Offset of this region inside the backing.
    pub gpu_offset: u64,
    /// `region_id` of the canonical mapping; 0 means this region itself.
    pub alias_of: u64,
    pub coherence: u32,
    pub map_state: u32,
    pub region_id: u64,
    pub epoch: u64,
}

pub const NVA_MEM_REGION_VERSION: u32 = 1;

pub const NVA_MEM_KIND_UNKNOWN: u32 = 0;
pub const NVA_MEM_KIND_CODE: u32 = 1;
pub const NVA_MEM_KIND_DATA: u32 = 2;
pub const NVA_MEM_KIND_HEAP: u32 = 3;
pub const NVA_MEM_KIND_STACK: u32 = 4;
pub const NVA_MEM_KIND_TLS: u32 = 5;
pub const NVA_MEM_KIND_GATE: u32 = 6;
pub const NVA_MEM_KIND_NVN_TABLE: u32 = 7;
pub const NVA_MEM_KIND_SHADER: u32 = 8;
pub const NVA_MEM_KIND_ROMFS: u32 = 9;
pub const NVA_MEM_KIND_GPU: u32 = 10;
pub const NVA_MEM_KIND_NVMAP: u32 = 11;

pub const NVA_MEM_OWNER_NONE: u32 = 0;
pub const NVA_MEM_OWNER_LOADER: u32 = 1;
pub const NVA_MEM_OWNER_HOST: u32 = 2;
pub const NVA_MEM_OWNER_ENGINE: u32 = 3;
pub const NVA_MEM_OWNER_PLATFORM: u32 = 4;

pub const NVA_MEM_PERM_R: u32 = 1;
pub const NVA_MEM_PERM_W: u32 = 2;
pub const NVA_MEM_PERM_X: u32 = 4;

pub const NVA_MEM_COHERENCE_UNTRACKED: u32 = 0;
pub const NVA_MEM_COHERENCE_HOST_FLUSHED: u32 = 1;
pub const NVA_MEM_COHERENCE_IMMUTABLE: u32 = 2;

pub const NVA_MEM_MAP_NONE: u32 = 0;
pub const NVA_MEM_MAP_CPU: u32 = 1;
pub const NVA_MEM_MAP_GPU: u32 = 2;
pub const NVA_MEM_MAP_CPU_GPU: u32 = 3;

/// 16 B.
#[repr(C)]
pub struct nva_fence {
    pub serial: u64,
    pub domain: u32,
    pub gen: u32,
}

pub const NVA_FENCE_VERSION: u32 = 1;

// ------------------------------------------------------- host services

/// 48 B.
#[repr(C)]
pub struct nva_host_services {
    pub size: u32,
    pub version: u32,
    pub user: *mut c_void,
    pub log: Option<
        unsafe extern "C" fn(user: *mut c_void, level: u32, tag: *const c_char, msg: *const c_char),
    >,
    pub mem_query: Option<
        unsafe extern "C" fn(
            user: *mut c_void,
            va: u64,
            size: u64,
            out: *mut nva_mem_region,
            out_off: *mut u64,
            out_covered: *mut u64,
        ) -> nva_status,
    >,
    pub on_device_lost: Option<unsafe extern "C" fn(user: *mut c_void, reason: u32, serial: u64)>,
    /// Engine only notifies; it never aborts or exits on its own.
    pub fatal: Option<
        unsafe extern "C" fn(user: *mut c_void, reason: u32, slot: u32, name: *const c_char),
    >,
}

pub const NVA_HOST_SERVICES_VERSION: u32 = 2;

// --------------------------------------------------------- shader cache

/// 32 B; owned by runtime, borrowed by engine.
#[repr(C)]
pub struct nva_shader_cache {
    pub size: u32,
    pub version: u32,
    pub format_version: u32,
    pub reserved: u32,
    pub data: *const c_void,
    pub data_size: u64,
}

pub const NVA_SHADER_CACHE_VERSION: u32 = 1;

// ---------------------------------------------------------------- gfx

/// 32 B.
#[repr(C)]
pub struct nva_gfx_desc {
    pub size: u32,
    pub version: u32,
    pub flags: u32,
    pub reserved: u32,
    /// Must be non-NULL; lives as long as the gfx object.
    pub host: *const nva_host_services,
    /// May be NULL; lives as long as the gfx object.
    pub shader_cache: *const nva_shader_cache,
}

pub const NVA_GFX_DESC_VERSION: u32 = 2;

/// 56 B.
#[repr(C)]
pub struct nva_gfx_caps {
    pub size: u32,
    pub version: u32,
    pub max_color_attachments: u32,
    pub max_viewports: u32,
    pub max_push_constants: u32,
    pub max_msaa_samples: u32,
    pub flags: u32,
    pub reserved: u32,
    pub max_memory_allocation_size: u64,
    pub max_memory_allocation_count: u32,
    pub reserved2: u32,
    pub min_imported_host_pointer_alignment: u64,
}

pub const NVA_GFX_CAPS_VERSION: u32 = 1;
pub const NVA_GFX_CAP_VULKAN_11: u32 = 0x1;
pub const NVA_GFX_CAP_CONDITIONAL_QUERY: u32 = 0x2;

/// 56 B. Mirrors the frozen v1 layout: the nine device counters proposed in
/// `docs/GATES.md` §2.4 are a separate ABI append (spec #37 D5), not this ticket.
#[repr(C)]
pub struct nva_stats {
    pub size: u32,
    pub version: u32,
    pub unsupported_calls: u32,
    pub unknown_resolves: u32,
    pub stub_aborts: u32,
    pub sticky_error: u32,
    pub last_serial: u32,
    pub flags: u32,
    pub frames: u64,
    pub draw_calls: u64,
    pub submits: u64,
}

pub const NVA_STATS_VERSION: u32 = 1;

// ---------------------------------------------------------------- NVN

/// 16 B; engine-owned.
#[repr(C)]
pub struct nva_nvn_slot {
    /// Incremented by the guest thunk.
    pub count: u64,
    /// Host implementation address.
    pub target: u64,
}

pub const NVA_NVN_RET_VOID: u32 = 0;
pub const NVA_NVN_RET_BOOL8: u32 = 1;
pub const NVA_NVN_RET_I32: u32 = 2;
pub const NVA_NVN_RET_U64: u32 = 3;
pub const NVA_NVN_RET_F32: u32 = 4;
pub const NVA_NVN_RET_PTR: u32 = 5;
pub const NVA_NVN_RET_STRUCT16: u32 = 6;
pub const NVA_NVN_RET_STRUCT_GT16: u32 = 7;
pub const NVA_NVN_RET_UNKNOWN: u32 = 0xFF;

pub const NVA_NVN_STUB_IMPL: u32 = 0;
pub const NVA_NVN_STUB_TYPED_ZERO: u32 = 1;
pub const NVA_NVN_STUB_ABORT: u32 = 2;
pub const NVA_NVN_STUB_UNKNOWN: u32 = 3;

/// 20 B; versioned through `nva_nvn_table.version`.
#[repr(C)]
pub struct nva_nvn_slot_meta {
    pub slot: u32,
    pub ret_class: u16,
    pub arg_gpr: u16,
    pub arg_fpr: u16,
    pub stack_bytes: u16,
    pub stub_class: u32,
    pub flags: u32,
}

/// 56 B; engine-owned, borrowed until `gfx_destroy`.
#[repr(C)]
pub struct nva_nvn_table {
    pub size: u32,
    pub version: u32,
    /// 523.
    pub slot_count: u32,
    /// 16.
    pub entry_size: u32,
    /// `const nva_nvn_slot *`, not a guest VA.
    pub slots: u64,
    /// `const char *const *`.
    pub names: u64,
    pub names_hash: u64,
    pub flags: u32,
    pub reserved: u32,
    /// `const nva_nvn_slot_meta *`; appended in table version 2.
    pub meta: u64,
}

pub const NVA_NVN_TABLE_VERSION: u32 = 2;
pub const NVA_NVN_TABLE_DIRECT: u32 = 0x0;
pub const NVA_NVN_TABLE_GATE_TLS: u32 = 0x1;
pub const NVA_NVN_TABLE_LSE: u32 = 0x2;

/// 16 B.
#[repr(C)]
pub struct nva_nvn_resolve_result {
    pub target: u64,
    pub slot: u32,
    pub flags: u32,
}

pub const NVA_NVN_RESOLVE_KNOWN: u32 = 0x1;
pub const NVA_NVN_RESOLVE_UNKNOWN: u32 = 0x2;
pub const NVA_NVN_RESOLVE_GATE: u32 = 0x4;

// --------------------------------------------------------------- host

/// 48 B.
#[repr(C)]
pub struct nva_thread_desc {
    pub size: u32,
    pub version: u32,
    pub priority: u32,
    pub flags: u32,
    pub stack_size: u64,
    pub tls_size: u64,
    pub entry: u64,
    pub arg: u64,
}

pub const NVA_THREAD_DESC_VERSION: u32 = 1;

/// 32 B.
#[repr(C)]
pub struct nva_host_desc {
    pub size: u32,
    pub version: u32,
    pub flags: u32,
    pub reserved: u32,
    /// Private directory, POSIX path.
    pub data_dir: *const c_char,
    /// Platform shell implementation; may be NULL.
    pub platform: *const nva_platform,
}

pub const NVA_HOST_DESC_VERSION: u32 = 1;

// --------------------------------------------------------------- 平台壳

/// 24 B.
#[repr(C)]
pub struct nva_audio_config {
    pub size: u32,
    pub version: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub format: u32,
    pub flags: u32,
}

pub const NVA_AUDIO_CONFIG_VERSION: u32 = 1;
pub const NVA_AUDIO_FORMAT_I16: u32 = 1;

/// 80 B; implemented by the shell, borrowed by runtime until app destroy.
#[repr(C)]
pub struct nva_platform {
    pub size: u32,
    pub version: u32,
    pub user: *mut c_void,
    pub audio_open: Option<
        unsafe extern "C" fn(
            user: *mut c_void,
            cfg: *const nva_audio_config,
            out_stream: *mut u64,
        ) -> nva_status,
    >,
    pub audio_write: Option<
        unsafe extern "C" fn(
            user: *mut c_void,
            stream: u64,
            data: *const c_void,
            bytes: u32,
        ) -> nva_status,
    >,
    pub audio_flush: Option<unsafe extern "C" fn(user: *mut c_void, stream: u64) -> nva_status>,
    pub audio_close: Option<unsafe extern "C" fn(user: *mut c_void, stream: u64) -> nva_status>,
    pub audio_set_volume:
        Option<unsafe extern "C" fn(user: *mut c_void, stream: u64, volume: f32) -> nva_status>,
    pub rumble: Option<
        unsafe extern "C" fn(
            user: *mut c_void,
            low: u32,
            high: u32,
            duration_ms: u32,
        ) -> nva_status,
    >,
    pub clock_now_ns: Option<unsafe extern "C" fn(user: *mut c_void, clock_id: u32) -> u64>,
    pub log: Option<
        unsafe extern "C" fn(user: *mut c_void, level: u32, tag: *const c_char, msg: *const c_char),
    >,
}

pub const NVA_PLATFORM_VERSION: u32 = 1;

/// 32 B.
#[repr(C)]
pub struct nva_input_event {
    pub size: u32,
    pub version: u32,
    /// The C header calls this field `type`, a Rust keyword; the layout is
    /// unchanged.
    pub r#type: u32,
    pub code: u32,
    pub value: i32,
    pub flags: u32,
    pub timestamp_ns: u64,
}

pub const NVA_INPUT_EVENT_VERSION: u32 = 1;
pub const NVA_INPUT_KEY: u32 = 1;
pub const NVA_INPUT_AXIS: u32 = 2;
pub const NVA_INPUT_TOUCH: u32 = 3;
pub const NVA_INPUT_GYRO: u32 = 4;
pub const NVA_INPUT_RUMBLE: u32 = 5;

/// 48 B.
#[repr(C)]
pub struct nva_app_callbacks {
    pub size: u32,
    pub version: u32,
    pub user: *mut c_void,
    pub on_surface_ready:
        Option<unsafe extern "C" fn(user: *mut c_void, generation: u32, width: u32, height: u32)>,
    pub on_surface_lost: Option<unsafe extern "C" fn(user: *mut c_void, generation: u32)>,
    pub on_device_lost: Option<unsafe extern "C" fn(user: *mut c_void, reason: u32, serial: u64)>,
    pub on_fatal:
        Option<unsafe extern "C" fn(user: *mut c_void, reason: u32, message: *const c_char)>,
}

pub const NVA_APP_CALLBACKS_VERSION: u32 = 1;

/// 40 B.
#[repr(C)]
pub struct nva_app_desc {
    pub size: u32,
    pub version: u32,
    pub flags: u32,
    pub reserved: u32,
    pub data_dir: *const c_char,
    pub callbacks: *const nva_app_callbacks,
    pub platform: *const nva_platform,
}

pub const NVA_APP_DESC_VERSION: u32 = 1;

// ---------------------------------------------------------------- SVC

/// 96 B; filled by the gate assembly, read-only in host, on the guest thread
/// stack.
#[repr(C)]
pub struct nva_svc_frame {
    pub size: u32,
    pub version: u32,
    /// x0..x8: x0 returns, x1..x7 are arguments, x8 is an indirect result.
    pub x: [u64; 9],
    /// Guest TLS before entering host; read-only.
    pub tpidr_el0: u64,
    pub gate: u32,
    pub flags: u32,
}

pub const NVA_SVC_FRAME_VERSION: u32 = 1;

// ---------------------------------------------------- layout assertions
//
// Mirror of `include/abi_layout_check.c`: same 19 `sizeof` checks, same 52
// `offsetof` checks and the same 5 version checks. These are `const` items, so a
// wrong size or offset is a compile error (E0080), never a runtime panic, and
// the crate does not need to be run to enforce the contract (#37 D3).
//
// Coverage boundary: the C file asserts only 5 of the 17 version macros, so this
// mirror does too. Every other constant above is still defined exactly as the C
// header defines it; it is simply not re-asserted on either side.

// `nva_sa!` mirrors the C file's `NVA_SA` macro: two arguments assert a size,
// three assert a field offset.
macro_rules! nva_sa {
    ($t:ty, $n:expr) => {
        const _: () = assert!(
            core::mem::size_of::<$t>() == $n,
            concat!(stringify!($t), " size")
        );
    };
    ($t:ty, $f:ident, $n:expr) => {
        const _: () = assert!(
            core::mem::offset_of!($t, $f) == $n,
            concat!(stringify!($t), ".", stringify!($f))
        );
    };
}

// ---- nva_mem_region: 104 B ----
nva_sa!(nva_mem_region, 104);
nva_sa!(nva_mem_region, guest_va, 8);
nva_sa!(nva_mem_region, size_bytes, 16);
nva_sa!(nva_mem_region, kind, 24);
nva_sa!(nva_mem_region, alloc_handle, 40);
nva_sa!(nva_mem_region, cpu_ptr, 48);
nva_sa!(nva_mem_region, gpu_handle, 56);
nva_sa!(nva_mem_region, gpu_offset, 64);
nva_sa!(nva_mem_region, alias_of, 72);
nva_sa!(nva_mem_region, coherence, 80);
nva_sa!(nva_mem_region, region_id, 88);
nva_sa!(nva_mem_region, epoch, 96);

// ---- nva_fence: 16 B ----
nva_sa!(nva_fence, 16);
nva_sa!(nva_fence, domain, 8);
nva_sa!(nva_fence, gen, 12);

// ---- nva_host_services: 48 B, fatal at offset 40 ----
nva_sa!(nva_host_services, 48);
nva_sa!(nva_host_services, user, 8);
nva_sa!(nva_host_services, log, 16);
nva_sa!(nva_host_services, mem_query, 24);
nva_sa!(nva_host_services, on_device_lost, 32);
nva_sa!(nva_host_services, fatal, 40);

// ---- nva_shader_cache: 32 B ----
nva_sa!(nva_shader_cache, 32);
nva_sa!(nva_shader_cache, data, 16);
nva_sa!(nva_shader_cache, data_size, 24);

// ---- nva_gfx_desc: 32 B ----
nva_sa!(nva_gfx_desc, 32);
nva_sa!(nva_gfx_desc, host, 16);
nva_sa!(nva_gfx_desc, shader_cache, 24);

// ---- nva_gfx_caps: 56 B ----
nva_sa!(nva_gfx_caps, 56);
nva_sa!(nva_gfx_caps, max_memory_allocation_size, 32);
nva_sa!(nva_gfx_caps, min_imported_host_pointer_alignment, 48);

// ---- nva_stats: 56 B ----
nva_sa!(nva_stats, 56);
nva_sa!(nva_stats, frames, 32);

// ---- NVN ----
nva_sa!(nva_nvn_slot, 16);
nva_sa!(nva_nvn_slot, target, 8);

nva_sa!(nva_nvn_slot_meta, 20);
nva_sa!(nva_nvn_slot_meta, ret_class, 4);
nva_sa!(nva_nvn_slot_meta, stack_bytes, 10);
nva_sa!(nva_nvn_slot_meta, stub_class, 12);
nva_sa!(nva_nvn_slot_meta, flags, 16);

nva_sa!(nva_nvn_table, 56);
nva_sa!(nva_nvn_table, slot_count, 8);
nva_sa!(nva_nvn_table, slots, 16);
nva_sa!(nva_nvn_table, names, 24);
nva_sa!(nva_nvn_table, names_hash, 32);
nva_sa!(nva_nvn_table, flags, 40);
nva_sa!(nva_nvn_table, meta, 48);

nva_sa!(nva_nvn_resolve_result, 16);
nva_sa!(nva_nvn_resolve_result, slot, 8);
nva_sa!(nva_nvn_resolve_result, flags, 12);

// ---- host ----
nva_sa!(nva_thread_desc, 48);
nva_sa!(nva_thread_desc, stack_size, 16);
nva_sa!(nva_thread_desc, entry, 32);

nva_sa!(nva_host_desc, 32);
nva_sa!(nva_host_desc, data_dir, 16);
nva_sa!(nva_host_desc, platform, 24);

// ---- 平台壳 ----
nva_sa!(nva_audio_config, 24);
nva_sa!(nva_platform, 80);
nva_sa!(nva_platform, audio_open, 16);
nva_sa!(nva_platform, log, 72);

nva_sa!(nva_input_event, 32);
nva_sa!(nva_input_event, timestamp_ns, 24);

nva_sa!(nva_app_callbacks, 48);
nva_sa!(nva_app_callbacks, on_fatal, 40);

nva_sa!(nva_app_desc, 40);
nva_sa!(nva_app_desc, callbacks, 24);
nva_sa!(nva_app_desc, platform, 32);

// ---- SVC ----
nva_sa!(nva_svc_frame, 96);
nva_sa!(nva_svc_frame, x, 8);
nva_sa!(nva_svc_frame, tpidr_el0, 80);
nva_sa!(nva_svc_frame, gate, 88);
nva_sa!(nva_svc_frame, flags, 92);

// ---- 版本宏必须与 C 侧一致 ----
const _: () = assert!(NVA_ABI_VERSION == 1, "NVA_ABI_VERSION");
const _: () = assert!(NVA_HOST_SERVICES_VERSION == 2, "NVA_HOST_SERVICES_VERSION");
const _: () = assert!(NVA_GFX_DESC_VERSION == 2, "NVA_GFX_DESC_VERSION");
const _: () = assert!(NVA_NVN_TABLE_VERSION == 2, "NVA_NVN_TABLE_VERSION");
const _: () = assert!(NVA_ERR_TIMEOUT == 8, "NVA_ERR_TIMEOUT");

/// The one test that exposes the layout contract.
///
/// The assertions above are `const` items, evaluated while this crate builds, so
/// reaching this test at all means the mirror has not drifted from
/// `include/abi_layout_check.c`. The body stays empty on purpose: runtime layout
/// probing is prohibited (#37 D3), and a runtime comparison here would duplicate
/// the compile-time check.
#[test]
fn layout_sizes() {}
