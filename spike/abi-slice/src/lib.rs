//! #30 ABI spike host/engine.
//!
//! The C ABI subset lives in `include/nva_abi.h`. The NVN-shaped entries are
//! reached through `nvnBootstrapLoader(name)` / `nva_nvn_resolve(table, name)`,
//! which return a guest-callable thunk. Thunks increment a {count,target} slot
//! and branch to the Rust implementation; the TLS gate variant is in
//! `src/thunks_aarch64.S`.
//!
//! This is a spike: one engine instance, one thread, no real NVN object model.

use core::ffi::{c_char, c_void, CStr};
use core::mem::size_of;
use core::ptr;
use core::slice;
use std::ffi::CString;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Mutex;

#[cfg(feature = "vk")]
mod vk;
#[cfg(not(feature = "vk"))]
#[path = "vk_mock.rs"]
mod vk;

const SLOT_COUNT: usize = 14;
const NVN_MAX_OPS: usize = 16;

const MAGIC_DEVICE: u64 = 0x4e56_4e44_4556_3031; // "NVNDEV01"
const MAGIC_BUILDER: u64 = 0x4e56_4e42_5549_3031; // "NVNBUI01"
const MAGIC_CMDBUF: u64 = 0x4e56_4e43_4d44_3031; // "NVNCMD01"
const MAGIC_QUEUE: u64 = 0x4e56_4e51_5545_3031; // "NVNQUE01"
const MAGIC_SAMPLER: u64 = 0x4e56_4e53_4d50_3031; // "NVNSMP01"
const MAGIC_BUFFER: u64 = 0x4e56_4e42_4646_3031; // "NVNBUF01"

const NVA_OK: u32 = 0;
const NVA_ERR_INVALID_ARGUMENT: u32 = 1;
const NVA_ERR_UNSUPPORTED: u32 = 2;

const TABLE_VERSION: u32 = 1;
const TABLE_ENTRY_SIZE: u32 = 16;
const TABLE_GATE_TLS: u32 = 0x1;
const TABLE_LSE: u32 = 0x2;

const RESOLVE_KNOWN: u32 = 0x1;
const RESOLVE_UNKNOWN: u32 = 0x2;
const RESOLVE_GATE: u32 = 0x4;

// Keep this order stable: the guest resolves by name, but the spike also
// checks slot order in nvn-table.json.
const SLOT_NAMES: [&str; SLOT_COUNT] = [
    "nvnDeviceBuilderSetDefaults",
    "nvnDeviceInitialize",
    "nvnDeviceGetProcAddress",
    "nvnCommandBufferSetDepthRange",
    "nvnCommandBufferClearColor",
    "nvnCommandBufferDrawArrays",
    "nvnQueueSubmitCommands",
    "nvnQueueFinish",
    "nvnSamplerGetLodBias",
    "nvnBufferGetAddress",
    "nvnCommandBufferDispatchCompute",
    "nvnDeviceGetCurrentTimestampInNanoseconds",
    "nvaSpikeTlsProbe",
    "nvaSpikeStackProbe",
];

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NvnSlot {
    pub count: u64,
    pub target: u64,
}

#[repr(C)]
pub struct NvnTable {
    pub size: u32,
    pub version: u32,
    pub slot_count: u32,
    pub entry_size: u32,
    pub slots: u64,
    pub names: u64,
    pub names_hash: u64,
    pub flags: u32,
    pub reserved: u32,
}

#[repr(C)]
pub struct NvnResolveResult {
    pub target: u64,
    pub slot: u32,
    pub flags: u32,
}

#[no_mangle]
pub static mut g_nvn_slots: [NvnSlot; SLOT_COUNT] = [NvnSlot { count: 0, target: 0 }; SLOT_COUNT];

#[cfg(target_arch = "aarch64")]
#[no_mangle]
pub static mut g_host_tls: u64 = 0;

// ---------------------------------------------------------------------------
// C ABI structs (v1 shapes)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostServices {
    pub size: u32,
    pub version: u32,
    pub user: *mut c_void,
    pub log: Option<unsafe extern "C" fn(*mut c_void, u32, *const c_char, *const c_char)>,
    pub mem_query: Option<
        unsafe extern "C" fn(
            *mut c_void,
            u64,
            u64,
            *mut MemRegion,
            *mut u64,
            *mut u64,
        ) -> u32,
    >,
    pub on_device_lost: Option<unsafe extern "C" fn(*mut c_void, u32, u64)>,
    pub shader_lookup: Option<unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> u32>,
}

#[repr(C)]
pub struct GfxDesc {
    pub size: u32,
    pub version: u32,
    pub flags: u32,
    pub host: *const HostServices,
}

#[repr(C)]
pub struct MemRegion {
    _bytes: [u8; 104],
}

#[repr(C)]
pub struct SpikeState {
    pub size: u32,
    pub version: u32,
    pub ok: u32,
    pub tls_gate: u32,
    pub validation_available: u32,
    pub validation_errors: u32,
    pub unsupported_calls: u32,
    pub unknown_resolves: u32,
    pub stub_aborts: u32,
    pub fence_wait_ok: u32,
    pub last_serial: u32,
    pub pixel_rgba: u32,
    pub depth_bits_near: u32,
    pub depth_bits_far: u32,
    pub tls_at_last_entry: u64,
    pub host_tls: u64,
    pub slot_counts: [u64; 16],
    pub backend: [u8; 64],
    pub device: [u8; 128],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NvaPair {
    pub a: u64,
    pub b: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NvaTriple {
    pub a: u64,
    pub b: u64,
    pub c: u64,
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

struct State {
    depth_bits_near: u32,
    depth_bits_far: u32,
    unsupported_calls: u32,
    unknown_resolves: u32,
    stub_aborts: u32,
    fence_wait_ok: u32,
    last_serial: u32,
    pixel_rgba: u32,
    tls_at_last_entry: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            depth_bits_near: 0,
            depth_bits_far: 0,
            unsupported_calls: 0,
            unknown_resolves: 0,
            stub_aborts: 0,
            fence_wait_ok: 0,
            last_serial: 0,
            pixel_rgba: 0,
            tls_at_last_entry: 0,
        }
    }
}

struct Engine {
    table: NvnTable,
    _names: Vec<CString>,
    _name_ptrs: Box<[*const c_char]>,
    host: HostServices,
    state: Mutex<State>,
    vk: Mutex<Option<vk::Vk>>,
    tls_gate: bool,
}

static GLOBAL: AtomicPtr<Engine> = AtomicPtr::new(ptr::null_mut());

unsafe fn engine() -> Option<&'static Engine> {
    let p = GLOBAL.load(Ordering::Acquire);
    if p.is_null() {
        None
    } else {
        Some(&*p)
    }
}

unsafe fn engine_mut() -> Option<&'static mut Engine> {
    let p = GLOBAL.load(Ordering::Acquire);
    if p.is_null() {
        None
    } else {
        Some(&mut *p)
    }
}

impl Engine {
    fn log(&self, level: u32, tag: &str, msg: &str) {
        if let Some(f) = self.host.log {
            let tag = CString::new(tag).unwrap_or_else(|_| CString::new("?").unwrap());
            let msg = CString::new(msg).unwrap_or_else(|_| CString::new("?").unwrap());
            unsafe { f(self.host.user, level, tag.as_ptr(), msg.as_ptr()) };
        }
    }
}

fn base_table_flags() -> u32 {
    if cfg!(target_arch = "aarch64") {
        TABLE_LSE
    } else {
        0
    }
}

unsafe fn capture_host_tls() {
    #[cfg(target_arch = "aarch64")]
    {
        use core::arch::asm;
        let v: u64;
        asm!("mrs {}, tpidr_el0", out(reg) v, options(nomem, nostack, preserves_flags));
        g_host_tls = v;
    }
}

// ---------------------------------------------------------------------------
// Thunk addresses
// ---------------------------------------------------------------------------

unsafe extern "C" {
    fn nvn_thunk_0();
    fn nvn_thunk_1();
    fn nvn_thunk_2();
    fn nvn_thunk_3();
    fn nvn_thunk_4();
    fn nvn_thunk_5();
    fn nvn_thunk_6();
    fn nvn_thunk_7();
    fn nvn_thunk_8();
    fn nvn_thunk_9();
    fn nvn_thunk_10();
    fn nvn_thunk_11();
    fn nvn_thunk_12();
    fn nvn_thunk_13();
}

static DIRECT_THUNKS: [unsafe extern "C" fn(); SLOT_COUNT] = [
    nvn_thunk_0,
    nvn_thunk_1,
    nvn_thunk_2,
    nvn_thunk_3,
    nvn_thunk_4,
    nvn_thunk_5,
    nvn_thunk_6,
    nvn_thunk_7,
    nvn_thunk_8,
    nvn_thunk_9,
    nvn_thunk_10,
    nvn_thunk_11,
    nvn_thunk_12,
    nvn_thunk_13,
];

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    fn nvn_gate_0();
    fn nvn_gate_1();
    fn nvn_gate_2();
    fn nvn_gate_3();
    fn nvn_gate_4();
    fn nvn_gate_5();
    fn nvn_gate_6();
    fn nvn_gate_7();
    fn nvn_gate_8();
    fn nvn_gate_9();
    fn nvn_gate_10();
    fn nvn_gate_11();
    fn nvn_gate_12();
    fn nvn_gate_13();
}

#[cfg(target_arch = "aarch64")]
static GATE_THUNKS: [unsafe extern "C" fn(); SLOT_COUNT] = [
    nvn_gate_0,
    nvn_gate_1,
    nvn_gate_2,
    nvn_gate_3,
    nvn_gate_4,
    nvn_gate_5,
    nvn_gate_6,
    nvn_gate_7,
    nvn_gate_8,
    nvn_gate_9,
    nvn_gate_10,
    nvn_gate_11,
    nvn_gate_12,
    nvn_gate_13,
];

fn thunk_addr(slot: usize, gate: bool) -> u64 {
    #[cfg(target_arch = "aarch64")]
    {
        if gate {
            return GATE_THUNKS[slot] as usize as u64;
        }
    }
    let _ = gate;
    DIRECT_THUNKS[slot] as usize as u64
}

fn impl_addr(slot: usize) -> u64 {
    let p: *const () = match slot {
        0 => impl_builder_set_defaults as *const (),
        1 => impl_device_initialize as *const (),
        2 => impl_device_get_proc_address as *const (),
        3 => impl_set_depth_range as *const (),
        4 => impl_clear_color as *const (),
        5 => impl_draw_arrays as *const (),
        6 => impl_queue_submit as *const (),
        7 => impl_queue_finish as *const (),
        8 => impl_sampler_get_lod_bias as *const (),
        9 => impl_buffer_get_address as *const (),
        10 => impl_compute_abort as *const (),
        11 => impl_timestamp_zero as *const (),
        12 => impl_tls_probe as *const (),
        13 => impl_stack_probe as *const (),
        _ => 0 as *const (),
    };
    p as u64
}

#[no_mangle]
pub extern "C" fn nvn_unknown_stub() -> u64 {
    0
}

// ---------------------------------------------------------------------------
// C ABI: create/destroy/table/resolve
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn nva_gfx_create(desc: *const GfxDesc, out: *mut *mut Engine) -> u32 {
    capture_host_tls();
    if out.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    out.write(ptr::null_mut());
    if desc.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let d = &*desc;
    if d.size < size_of::<GfxDesc>() as u32 || d.version != 1 || d.host.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let host = *d.host;
    if host.size < size_of::<HostServices>() as u32 || host.version != 1 {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    if !GLOBAL.load(Ordering::Acquire).is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }

    let names: Vec<CString> = SLOT_NAMES
        .iter()
        .map(|n| CString::new(*n).expect("static name"))
        .collect();
    let ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
    let name_ptrs = ptrs.into_boxed_slice();

    let mut hash: u64 = 0xcbf29ce484222325;
    for n in &names {
        for &b in n.as_bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0;
        hash = hash.wrapping_mul(0x100000001b3);
    }

    let mut flags = base_table_flags();
    let gate_env = std::env::var_os("NVA_SPIKE_TLS_GATE").is_some();
    if cfg!(target_arch = "aarch64") && gate_env {
        flags |= TABLE_GATE_TLS;
    }

    for i in 0..SLOT_COUNT {
        g_nvn_slots[i] = NvnSlot {
            count: 0,
            target: impl_addr(i),
        };
    }

    let table = NvnTable {
        size: size_of::<NvnTable>() as u32,
        version: TABLE_VERSION,
        slot_count: SLOT_COUNT as u32,
        entry_size: TABLE_ENTRY_SIZE,
        slots: ptr::addr_of!(g_nvn_slots) as u64,
        names: name_ptrs.as_ptr() as u64,
        names_hash: hash,
        flags,
        reserved: 0,
    };

    let engine = Box::new(Engine {
        table,
        _names: names,
        _name_ptrs: name_ptrs,
        host,
        state: Mutex::new(State::default()),
        vk: Mutex::new(None),
        tls_gate: flags & TABLE_GATE_TLS != 0,
    });
    let raw = Box::into_raw(engine);
    GLOBAL.store(raw, Ordering::Release);
    (*raw).log(
        1,
        "abi-slice",
        &format!(
            "nva_gfx_create: slots={} gate={}",
            SLOT_COUNT,
            (*raw).tls_gate
        ),
    );
    out.write(raw);
    NVA_OK
}

#[no_mangle]
pub unsafe extern "C" fn nva_gfx_destroy(gfx: *mut Engine) {
    if gfx.is_null() {
        return;
    }
    if GLOBAL.load(Ordering::Acquire) == gfx {
        GLOBAL.store(ptr::null_mut(), Ordering::Release);
    }
    drop(Box::from_raw(gfx));
}

#[no_mangle]
pub unsafe extern "C" fn nva_gfx_get_nvn_table(gfx: *mut Engine) -> *const NvnTable {
    if gfx.is_null() {
        return ptr::null();
    }
    ptr::addr_of!((*gfx).table)
}

unsafe fn lookup_slot(table: *const NvnTable, name: *const c_char) -> Option<usize> {
    if table.is_null() || name.is_null() {
        return None;
    }
    let t = &*table;
    if t.size < size_of::<NvnTable>() as u32
        || t.entry_size != TABLE_ENTRY_SIZE
        || t.slot_count == 0
        || t.slot_count > 1024
        || t.names == 0
    {
        return None;
    }
    let names = slice::from_raw_parts(t.names as *const *const c_char, t.slot_count as usize);
    let needle = CStr::from_ptr(name).to_bytes();
    for (i, &p) in names.iter().enumerate() {
        if !p.is_null() && CStr::from_ptr(p).to_bytes() == needle {
            return Some(i);
        }
    }
    None
}

#[no_mangle]
pub unsafe extern "C" fn nva_nvn_resolve(table: *const NvnTable, name: *const c_char) -> u64 {
    if let Some(slot) = lookup_slot(table, name) {
        return thunk_addr(slot, (*table).flags & TABLE_GATE_TLS != 0);
    }
    if let Some(e) = engine() {
        if let Ok(mut st) = e.state.lock() {
            st.unknown_resolves = st.unknown_resolves.saturating_add(1);
        }
        let n = if name.is_null() {
            "<null>".to_string()
        } else {
            CStr::from_ptr(name).to_string_lossy().into_owned()
        };
        e.log(2, "abi-slice", &format!("unknown NVN name: {n}"));
    }
    nvn_unknown_stub as *const () as u64
}

#[no_mangle]
pub unsafe extern "C" fn nva_nvn_resolve_ex(
    table: *const NvnTable,
    name: *const c_char,
    out: *mut NvnResolveResult,
) -> u32 {
    if out.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    if let Some(slot) = lookup_slot(table, name) {
        let gate = (*table).flags & TABLE_GATE_TLS != 0;
        out.write(NvnResolveResult {
            target: thunk_addr(slot, gate),
            slot: slot as u32,
            flags: RESOLVE_KNOWN | if gate { RESOLVE_GATE } else { 0 },
        });
        NVA_OK
    } else {
        out.write(NvnResolveResult {
            target: nvn_unknown_stub as *const () as u64,
            slot: u32::MAX,
            flags: RESOLVE_UNKNOWN,
        });
        if let Some(e) = engine() {
            if let Ok(mut st) = e.state.lock() {
                st.unknown_resolves = st.unknown_resolves.saturating_add(1);
            }
        }
        NVA_OK
    }
}

#[no_mangle]
pub unsafe extern "C" fn nvnBootstrapLoader(name: *const c_char) -> *mut c_void {
    match engine() {
        Some(e) => nva_nvn_resolve(ptr::addr_of!(e.table), name) as *mut c_void,
        None => nvn_unknown_stub as *const () as *mut c_void,
    }
}

#[no_mangle]
pub unsafe extern "C" fn nvnDeviceGetProcAddress(
    device: *const c_void,
    name: *const c_char,
) -> *mut c_void {
    if !device.is_null() && *(device as *const u64) != MAGIC_DEVICE {
        if let Some(e) = engine() {
            e.log(2, "abi-slice", "nvnDeviceGetProcAddress: bad device magic");
        }
        return nvn_unknown_stub as *const () as *mut c_void;
    }
    nvnBootstrapLoader(name)
}

// ---------------------------------------------------------------------------
// NVN-shaped implementations (thunk targets)
// ---------------------------------------------------------------------------

unsafe fn check_magic<T>(p: *const T, magic: u64, what: &str) -> bool {
    if p.is_null() || *(p as *const u64) != magic {
        if let Some(e) = engine() {
            e.log(2, "abi-slice", &format!("{what}: bad/null object"));
        }
        false
    } else {
        true
    }
}

unsafe extern "C" fn impl_builder_set_defaults(builder: *mut c_void) {
    if builder.is_null() {
        return;
    }
    ptr::write_bytes(builder as *mut u8, 0, 64);
    *(builder as *mut u64) = MAGIC_BUILDER;
}

unsafe extern "C" fn impl_device_initialize(device: *mut c_void, builder: *const c_void) -> u32 {
    if !check_magic(builder, MAGIC_BUILDER, "nvnDeviceInitialize(builder)") {
        return 0;
    }
    let Some(e) = engine() else { return 0 };
    let mut slot = match e.vk.lock() {
        Ok(g) => g,
        Err(_) => return 0,
    };
    if slot.is_none() {
        match vk::Vk::new(true) {
            Ok(v) => {
                let info = v.info();
                e.log(
                    1,
                    "abi-slice",
                    &format!(
                        "Vulkan device: {} ({}) validation={}",
                        info.device, info.backend, info.validation
                    ),
                );
                *slot = Some(v);
            }
            Err(err) => {
                e.log(2, "abi-slice", &format!("Vk::new failed: {err}"));
                return 0;
            }
        }
    }
    drop(slot);
    if !device.is_null() {
        *(device as *mut u64) = MAGIC_DEVICE;
    }
    1
}

unsafe extern "C" fn impl_device_get_proc_address(
    device: *const c_void,
    name: *const c_char,
) -> *mut c_void {
    if !device.is_null() && *(device as *const u64) != MAGIC_DEVICE {
        return nvn_unknown_stub as *const () as *mut c_void;
    }
    nvnBootstrapLoader(name)
}

unsafe fn push_op(cb: *mut NvnCommandBuffer, op: NvnOp) {
    let cb = &mut *cb;
    if cb.op_count as usize >= NVN_MAX_OPS {
        return;
    }
    cb.ops[cb.op_count as usize] = op;
    cb.op_count += 1;
}

unsafe extern "C" fn impl_set_depth_range(cb: *mut NvnCommandBuffer, near: f32, far: f32) {
    if !check_magic(cb, MAGIC_CMDBUF, "nvnCommandBufferSetDepthRange") {
        return;
    }
    let cb = &mut *cb;
    cb.depth_near = near;
    cb.depth_far = far;
    if let Some(e) = engine() {
        if let Ok(mut st) = e.state.lock() {
            st.depth_bits_near = near.to_bits();
            st.depth_bits_far = far.to_bits();
        }
        e.log(
            1,
            "abi-slice",
            &format!(
                "SetDepthRange near={:.6} ({:#010x}) far={:.6} ({:#010x})",
                near,
                near.to_bits(),
                far,
                far.to_bits()
            ),
        );
    }
    push_op(
        cb,
        NvnOp {
            kind: 3,
            a: 0,
            f: [near, far, 0.0, 0.0],
            i: [0, 0],
        },
    );
}

unsafe extern "C" fn impl_clear_color(
    cb: *mut NvnCommandBuffer,
    index: i32,
    rgba: *const f32,
    mask: i32,
) {
    if !check_magic(cb, MAGIC_CMDBUF, "nvnCommandBufferClearColor") || rgba.is_null() {
        return;
    }
    let c = slice::from_raw_parts(rgba, 4);
    push_op(
        cb,
        NvnOp {
            kind: 1,
            a: index as u32,
            f: [c[0], c[1], c[2], c[3]],
            i: [mask, 0],
        },
    );
}

unsafe extern "C" fn impl_draw_arrays(
    cb: *mut NvnCommandBuffer,
    primitive: i32,
    first: i32,
    count: i32,
) {
    if !check_magic(cb, MAGIC_CMDBUF, "nvnCommandBufferDrawArrays") {
        return;
    }
    push_op(
        cb,
        NvnOp {
            kind: 2,
            a: primitive as u32,
            f: [0.0; 4],
            i: [first, count],
        },
    );
}

unsafe extern "C" fn impl_queue_submit(
    queue: *mut c_void,
    count: i32,
    handles: *const *mut c_void,
) {
    if !check_magic(queue, MAGIC_QUEUE, "nvnQueueSubmitCommands") {
        return;
    }
    if count <= 0 || handles.is_null() {
        return;
    }
    let Some(e) = engine() else { return };

    let mut clear = [0.0f32; 4];
    let mut have_clear = false;
    let mut draw_first = 0i32;
    let mut draw_count = 0i32;

    for i in 0..count as usize {
        let h = *handles.add(i);
        if h.is_null() {
            continue;
        }
        let cb = &*(h as *const NvnCommandBuffer);
        if cb.magic != MAGIC_CMDBUF {
            continue;
        }
        let n = (cb.op_count as usize).min(NVN_MAX_OPS);
        for op in cb.ops[..n].iter() {
            match op.kind {
                1 => {
                    clear = op.f;
                    have_clear = true;
                }
                2 => {
                    draw_first = op.i[0];
                    draw_count = op.i[1];
                }
                3 => {}
                _ => {}
            }
        }
    }
    let clear = if have_clear { clear } else { [0.0, 0.0, 0.0, 1.0] };

    let mut guard = match e.vk.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let Some(v) = guard.as_mut() else {
        e.log(2, "abi-slice", "nvnQueueSubmitCommands before nvnDeviceInitialize");
        return;
    };
    match v.submit_offscreen(clear, draw_first, draw_count) {
        Ok(serial) => {
            let pixel = v.read_center().unwrap_or([0, 0, 0, 0]);
            let rgba = pixel[0] as u32
                | (pixel[1] as u32) << 8
                | (pixel[2] as u32) << 16
                | (pixel[3] as u32) << 24;
            let verr = v.validation_errors();
            drop(guard);
            if let Ok(mut st) = e.state.lock() {
                st.fence_wait_ok = 1;
                st.last_serial = serial;
                st.pixel_rgba = rgba;
            }
            e.log(
                1,
                "abi-slice",
                &format!(
                    "submit: serial={} draw=({},{}) clear=[{:.2},{:.2},{:.2},{:.2}] pixel={:#010x} vk_errors={}",
                    serial, draw_first, draw_count, clear[0], clear[1], clear[2], clear[3], rgba, verr
                ),
            );
        }
        Err(err) => {
            drop(guard);
            if let Ok(mut st) = e.state.lock() {
                st.fence_wait_ok = 0;
            }
            e.log(2, "abi-slice", &format!("submit failed: {err}"));
        }
    }
}

unsafe extern "C" fn impl_queue_finish(queue: *mut c_void) {
    if !check_magic(queue, MAGIC_QUEUE, "nvnQueueFinish") {
        return;
    }
}

unsafe extern "C" fn impl_sampler_get_lod_bias(sampler: *const NvnSampler) -> f32 {
    if !check_magic(sampler, MAGIC_SAMPLER, "nvnSamplerGetLodBias") {
        return 0.0;
    }
    (*sampler).lod_bias
}

unsafe extern "C" fn impl_buffer_get_address(buffer: *const NvnBuffer) -> u64 {
    if !check_magic(buffer, MAGIC_BUFFER, "nvnBufferGetAddress") {
        return 0;
    }
    (*buffer).address
}

unsafe extern "C" fn impl_compute_abort(_cb: *mut c_void, _x: i32, _y: i32, _z: i32) {
    if let Some(e) = engine() {
        if let Ok(mut st) = e.state.lock() {
            st.stub_aborts = st.stub_aborts.saturating_add(1);
        }
        e.log(
            4,
            "abi-slice",
            "UNIMPLEMENTED nvnCommandBufferDispatchCompute -> abort (sticky-fatal)",
        );
    }
    libc::abort();
}

unsafe extern "C" fn impl_timestamp_zero(_device: *const c_void) -> u64 {
    if let Some(e) = engine() {
        let first = if let Ok(mut st) = e.state.lock() {
            st.unsupported_calls = st.unsupported_calls.saturating_add(1);
            st.unsupported_calls == 1
        } else {
            false
        };
        if first {
            e.log(
                3,
                "abi-slice",
                "UNIMPLEMENTED nvnDeviceGetCurrentTimestampInNanoseconds -> typed 0 + sticky",
            );
        }
    }
    0
}

unsafe extern "C" fn impl_tls_probe() -> u64 {
    #[cfg(target_arch = "aarch64")]
    {
        use core::arch::asm;
        let v: u64;
        asm!("mrs {}, tpidr_el0", out(reg) v, options(nomem, nostack, preserves_flags));
        v
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        0
    }
}

unsafe extern "C" fn impl_stack_probe(
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    a6: u64,
    a7: u64,
    a8: u64,
    a9: u64,
) -> u64 {
    a0.wrapping_add(a1)
        .wrapping_add(a2)
        .wrapping_add(a3)
        .wrapping_add(a4)
        .wrapping_add(a5)
        .wrapping_add(a6)
        .wrapping_add(a7)
        .wrapping_add(a8)
        .wrapping_add(a9)
}

// ---------------------------------------------------------------------------
// Spike hooks
// ---------------------------------------------------------------------------

fn copy_c(dst: &mut [u8], s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&b[..n]);
    dst[n] = 0;
}

#[no_mangle]
pub unsafe extern "C" fn nva_spike_get_state(gfx: *mut Engine, out: *mut SpikeState) -> u32 {
    if out.is_null() || gfx.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let e = &*gfx;
    let mut s = SpikeState {
        size: size_of::<SpikeState>() as u32,
        version: 1,
        ok: 1,
        tls_gate: e.tls_gate as u32,
        validation_available: 0,
        validation_errors: 0,
        unsupported_calls: 0,
        unknown_resolves: 0,
        stub_aborts: 0,
        fence_wait_ok: 0,
        last_serial: 0,
        pixel_rgba: 0,
        depth_bits_near: 0,
        depth_bits_far: 0,
        tls_at_last_entry: g_host_tls_or_zero(),
        host_tls: g_host_tls_or_zero(),
        slot_counts: [0; 16],
        backend: [0; 64],
        device: [0; 128],
    };
    if let Ok(st) = e.state.lock() {
        s.unsupported_calls = st.unsupported_calls;
        s.unknown_resolves = st.unknown_resolves;
        s.stub_aborts = st.stub_aborts;
        s.fence_wait_ok = st.fence_wait_ok;
        s.last_serial = st.last_serial;
        s.pixel_rgba = st.pixel_rgba;
        s.depth_bits_near = st.depth_bits_near;
        s.depth_bits_far = st.depth_bits_far;
        s.tls_at_last_entry = st.tls_at_last_entry;
    }
    for i in 0..SLOT_COUNT {
        s.slot_counts[i] = ptr::read_volatile(ptr::addr_of!(g_nvn_slots[i].count));
    }
    if let Ok(g) = e.vk.lock() {
        if let Some(v) = g.as_ref() {
            let info = v.info();
            s.validation_available = info.validation as u32;
            s.validation_errors = v.validation_errors();
            copy_c(&mut s.backend, &info.backend);
            copy_c(&mut s.device, &info.device);
        }
    }
    out.write(s);
    NVA_OK
}

fn g_host_tls_or_zero() -> u64 {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { g_host_tls }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn nva_spike_read_pixel(gfx: *mut Engine, out: *mut u32) -> u32 {
    if out.is_null() || gfx.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    if let Ok(st) = (*gfx).state.lock() {
        *out = st.pixel_rgba;
        NVA_OK
    } else {
        NVA_ERR_INVALID_ARGUMENT
    }
}

#[no_mangle]
pub unsafe extern "C" fn nva_spike_set_tls_gate(gfx: *mut Engine, enable: u32) -> u32 {
    if gfx.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (gfx, enable);
        NVA_ERR_UNSUPPORTED
    }
    #[cfg(target_arch = "aarch64")]
    {
        let e = &mut *gfx;
        e.tls_gate = enable != 0;
        let mut flags = base_table_flags();
        if e.tls_gate {
            flags |= TABLE_GATE_TLS;
        }
        e.table.flags = flags;
        e.log(
            1,
            "abi-slice",
            &format!("TLS gate {}", if e.tls_gate { "on" } else { "off" }),
        );
        NVA_OK
    }
}

#[no_mangle]
pub unsafe extern "C" fn nva_spike_wait(_gfx: *mut Engine, _timeout_ns: u64) -> u32 {
    NVA_OK
}

#[no_mangle]
pub extern "C" fn nvaSpikeReturnPair() -> NvaPair {
    NvaPair {
        a: 0x1122_3344_5566_7788,
        b: 0x99aa_bbcc_ddee_ff00,
    }
}

#[no_mangle]
pub extern "C" fn nvaSpikeReturnTriple() -> NvaTriple {
    NvaTriple {
        a: 0x0102_0304_0506_0708,
        b: 0x1112_1314_1516_1718,
        c: 0x2122_2324_2526_2728,
    }
}

// ---------------------------------------------------------------------------
// Guest-owned object layouts (must match include/nvn_guest.h)
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct NvnOp {
    pub kind: u32,
    pub a: u32,
    pub f: [f32; 4],
    pub i: [i32; 2],
}

#[repr(C)]
pub struct NvnCommandBuffer {
    pub magic: u64,
    pub op_count: u32,
    pub state_flags: u32,
    pub depth_near: f32,
    pub depth_far: f32,
    pub reserved: u32,
    pub ops: [NvnOp; NVN_MAX_OPS],
}

#[repr(C)]
pub struct NvnSampler {
    pub magic: u64,
    pub lod_bias: f32,
    pub reserved: u32,
    pub pad: u64,
}

#[repr(C)]
pub struct NvnBuffer {
    pub magic: u64,
    pub address: u64,
}

// ---------------------------------------------------------------------------
// Compile-time layout checks (mirror include/abi_layout_check.c)
// ---------------------------------------------------------------------------
const _: () = {
    assert!(size_of::<NvnSlot>() == 16);
    assert!(size_of::<NvnTable>() == 48);
    assert!(size_of::<HostServices>() == 48);
    assert!(size_of::<GfxDesc>() == 24);
    assert!(size_of::<NvnOp>() == 32);
    assert!(size_of::<NvnCommandBuffer>() == 544);
    assert!(core::mem::offset_of!(NvnCommandBuffer, ops) == 28);
    assert!(size_of::<NvnSampler>() == 24);
    assert!(core::mem::offset_of!(NvnSampler, lod_bias) == 8);
    assert!(size_of::<NvnBuffer>() == 16);
    assert!(size_of::<SpikeState>() == 392);
};

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("thunks_x86_64.S"));

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("thunks_aarch64.S"));
