/*
 * guest.c — #30 ABI vertical slice guest (plain C11, no C++).
 *
 * The guest links the Rust cdylib libnvn_host.so in the same address space.
 * It creates an nva_gfx, borrows the NVN slot table, resolves NVN entry points
 * through nvnBootstrapLoader() / nva_nvn_resolve() / nvnDeviceGetProcAddress(),
 * and calls them through the host thunks.
 *
 * Modes (argv[1], default "all"):
 *   all            five assertions + AArch64 probes, SUMMARY, exit 0 iff clean
 *   gate           enable NVA_NVN_TABLE_GATE_TLS before resolving; x86_64 skips
 *   unimplemented  call the compute abort stub; the parent expects SIGABRT (134)
 *   stackgate      AArch64 only: gate thunk must shift SP and break stack args
 *
 * Output contract: one line per assertion, e.g.
 *   RESULT {"assertion":"name_lookup","ok":true}
 * and a final SUMMARY line for the non-aborting modes. Every line is flushed.
 *
 * AArch64 fake-TLS sections deliberately contain no libc calls: only the
 * inline-asm TPIDR_EL0 writes, the thunk call, and the TPIDR_EL0 read back.
 * Build this file with -fno-stack-protector -fno-omit-frame-pointer. For the
 * x18 probe, build the AArch64 guest with -ffixed-x18 and -DNVN_X18_RESERVED=1
 * (Android already reserves x18 and is detected via __ANDROID__).
 */

#include <inttypes.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "nva_abi.h"
#include "nvn_guest.h"

/* bionic/old glibc hide this under strict C11; declare it explicitly. */
extern int posix_memalign(void **memptr, size_t alignment, size_t size);

#ifdef __ANDROID__
#include <android/log.h>
#endif

/* The host and guest share these layouts; a mismatch is an ABI bug. */
_Static_assert(sizeof(nva_nvn_slot) == 16, "nva_nvn_slot must stay 16 bytes");
_Static_assert(sizeof(NVNop) == 32, "NVNop must stay 32 bytes");

/* ------------------------------------------------------------------------- */
/* Host services                                                             */
/* ------------------------------------------------------------------------- */

/* Set while the fake TPIDR_EL0 is installed.  A log from the host at that
 * point would enter libc TLS and crash, so drop it instead. */
static volatile int g_fake_tls_active;

static void guest_log(void *user, uint32_t level, const char *tag,
                      const char *msg)
{
    (void)user;
    if (g_fake_tls_active)
        return;

    printf("LOG [%" PRIu32 "] %s: %s\n", level, tag ? tag : "-",
           msg ? msg : "");
    fflush(stdout);
#ifdef __ANDROID__
    __android_log_print(ANDROID_LOG_INFO, tag ? tag : "nvn_guest", "%s",
                        msg ? msg : "");
#endif
}

static nva_status guest_mem_query(void *user, uint64_t va, uint64_t size,
                                  nva_mem_region *out, uint64_t *out_off,
                                  uint64_t *out_covered)
{
    (void)user;
    (void)va;
    (void)size;
    (void)out;
    (void)out_off;
    (void)out_covered;
    return NVA_ERR_UNSUPPORTED;
}

/* Engine-owned borrows: these must outlive nva_gfx_create(). */
static nva_host_services g_host;
static nva_gfx_desc g_desc;
static nva_gfx *g_gfx;
static const nva_nvn_table *g_table;
static NVNdevice g_dev;

/* ------------------------------------------------------------------------- */
/* Assertion plumbing and JSON                                               */
/* ------------------------------------------------------------------------- */

#if defined(__GNUC__)
#define PRINTF_LIKE(a, b) __attribute__((format(printf, a, b)))
#else
#define PRINTF_LIKE(a, b)
#endif

static const char *g_assertion = "init";
static int g_assertion_failures;
static char g_detail[512];
static char g_extra[2048];

static int g_failures;          /* failed individual checks */
static int g_failed_assertions; /* failed RESULT lines */
static int g_result_count;      /* RESULT lines emitted */

static void failf(const char *fmt, ...) PRINTF_LIKE(1, 2);

static void failf(const char *fmt, ...)
{
    if (g_assertion_failures == 0) {
        va_list ap;
        va_start(ap, fmt);
        (void)vsnprintf(g_detail, sizeof g_detail, fmt, ap);
        va_end(ap);
    }
    g_assertion_failures++;
}

#define CHECK(cond)                                                            \
    do {                                                                       \
        if (!(cond))                                                           \
            failf("line %d: check failed: %s", __LINE__, #cond);               \
    } while (0)

#define CHECK_MSG(cond, ...)                                                   \
    do {                                                                       \
        if (!(cond))                                                           \
            failf(__VA_ARGS__);                                                \
    } while (0)

static void assertion_begin(const char *name)
{
    g_assertion = name;
    g_assertion_failures = 0;
    g_detail[0] = '\0';
    g_extra[0] = '\0';
}

/* Escape into a pure-ASCII JSON string body.  Bytes >= 0x7f are escaped too so
 * an arbitrary Vulkan device name cannot produce invalid UTF-8 JSON. */
static void json_escape(char *dst, size_t cap, const char *src)
{
    size_t j = 0;

    if (cap == 0)
        return;
    if (src) {
        for (size_t i = 0; src[i] != '\0'; i++) {
            unsigned char c = (unsigned char)src[i];
            const char *rep = NULL;
            char ubuf[8];

            switch (c) {
            case '"':
                rep = "\\\"";
                break;
            case '\\':
                rep = "\\\\";
                break;
            case '\n':
                rep = "\\n";
                break;
            case '\r':
                rep = "\\r";
                break;
            case '\t':
                rep = "\\t";
                break;
            default:
                if (c < 0x20 || c >= 0x7f) {
                    (void)snprintf(ubuf, sizeof ubuf, "\\u%04x",
                                   (unsigned int)c);
                    rep = ubuf;
                }
                break;
            }
            if (rep) {
                size_t n = strlen(rep);
                if (j + n + 1 > cap)
                    break;
                memcpy(dst + j, rep, n);
                j += n;
            } else {
                if (j + 2 > cap)
                    break;
                dst[j++] = (char)c;
            }
        }
    }
    dst[j] = '\0';
}

static void emit_result(int ok)
{
    if (ok) {
        printf("RESULT {\"assertion\":\"%s\",\"ok\":true%s}\n", g_assertion,
               g_extra);
    } else {
        char detail[3200];
        json_escape(detail, sizeof detail, g_detail);
        printf("RESULT {\"assertion\":\"%s\",\"ok\":false,\"detail\":\"%s\"%s}\n",
               g_assertion, detail, g_extra);
    }
    fflush(stdout);
}

static int assertion_end(void)
{
    int ok = (g_assertion_failures == 0);

    emit_result(ok);
    g_result_count++;
    if (!ok) {
        g_failed_assertions++;
        g_failures += g_assertion_failures;
    }
    g_assertion_failures = 0;
    g_detail[0] = '\0';
    g_extra[0] = '\0';
    return ok;
}

static void emit_summary(const char *mode)
{
    uint32_t tls_gate = 0;
    uint32_t validation = 0;

    if (g_gfx) {
        nva_spike_state st;
        memset(&st, 0, sizeof st);
        st.size = (uint32_t)sizeof st;
        st.version = NVA_SPIKE_STATE_VERSION;
        if (nva_spike_get_state(g_gfx, &st) == NVA_OK) {
            tls_gate = st.tls_gate;
            validation = st.validation_available;
        }
    }

    printf("SUMMARY {\"mode\":\"%s\",\"ok\":%s,\"assertions\":%d,"
           "\"failed_assertions\":%d,\"failed_checks\":%d,"
           "\"tls_gate\":%" PRIu32 ",\"validation_available\":%" PRIu32 "}\n",
           mode, g_failures == 0 ? "true" : "false", g_result_count,
           g_failed_assertions, g_failures, tls_gate, validation);
    fflush(stdout);
}

/* ------------------------------------------------------------------------- */
/* Init, resolve, and state helpers                                          */
/* ------------------------------------------------------------------------- */

static nva_status query_state(nva_spike_state *st)
{
    memset(st, 0, sizeof *st);
    st->size = (uint32_t)sizeof *st;
    st->version = NVA_SPIKE_STATE_VERSION;
    return nva_spike_get_state(g_gfx, st);
}

static int get_state(nva_spike_state *st)
{
    nva_status s = query_state(st);

    if (s != NVA_OK) {
        failf("nva_spike_get_state -> %" PRIu32, s);
        return 0;
    }
    return 1;
}

static void *boot(const char *name)
{
    void *p = nvnBootstrapLoader(name);

    if (!p)
        failf("nvnBootstrapLoader(\"%s\") returned NULL", name);
    return p;
}

static void *resolve_table(const char *name)
{
    uint64_t va = nva_nvn_resolve(g_table, name);

    if (va == 0)
        failf("nva_nvn_resolve(\"%s\") returned 0", name);
    return (void *)(uintptr_t)va;
}

static void copy_cstr(char *dst, size_t dst_cap, const char *src,
                      size_t src_cap)
{
    size_t n = 0;

    while (n < src_cap && src[n] != '\0')
        n++;
    if (n >= dst_cap)
        n = dst_cap - 1;
    memcpy(dst, src, n);
    dst[n] = '\0';
}

/* Returns 1 on success.  On fatal failure it emits a single
 * RESULT {"assertion":"init",...} and returns 0; the caller adds SUMMARY. */
static int guest_init(void)
{
    uint32_t bad_names = 0;

    assertion_begin("init");

    memset(&g_host, 0, sizeof g_host);
    g_host.size = (uint32_t)sizeof g_host;
    g_host.version = NVA_HOST_SERVICES_VERSION;
    g_host.user = NULL;
    g_host.log = guest_log;
    g_host.mem_query = guest_mem_query;

    memset(&g_desc, 0, sizeof g_desc);
    g_desc.size = (uint32_t)sizeof g_desc;
    g_desc.version = NVA_GFX_DESC_VERSION;
    g_desc.flags = 0;
    g_desc.host = &g_host;

    nva_status s = nva_gfx_create(&g_desc, &g_gfx);
    if (s != NVA_OK) {
        failf("nva_gfx_create -> %" PRIu32, s);
        (void)assertion_end();
        return 0;
    }

    g_table = nva_gfx_get_nvn_table(g_gfx);
    if (!g_table) {
        failf("nva_gfx_get_nvn_table returned NULL");
        (void)assertion_end();
        return 0;
    }

    CHECK(g_table->size == (uint32_t)sizeof(nva_nvn_table));
    CHECK(g_table->version == NVA_NVN_TABLE_VERSION);
    CHECK(g_table->slot_count >= 14);
    CHECK(g_table->entry_size == (uint32_t)sizeof(nva_nvn_slot));
    CHECK(g_table->slots != 0);
    CHECK(g_table->names != 0);

    if (g_table->names) {
        const char *const *names =
            (const char *const *)(uintptr_t)g_table->names;
        uint32_t n = g_table->slot_count < 14 ? g_table->slot_count : 14;
        for (uint32_t i = 0; i < n; i++) {
            if (!names[i] || names[i][0] == '\0')
                bad_names++;
        }
    }
    CHECK_MSG(bad_names == 0, "%" PRIu32 " table slot(s) have NULL/empty names",
              bad_names);

    if (g_assertion_failures != 0) {
        (void)assertion_end();
        return 0;
    }

    /* Success is deliberately silent: only the five assertions and the
     * platform probes produce RESULT lines. */
    g_assertion_failures = 0;
    g_detail[0] = '\0';
    g_extra[0] = '\0';
    return 1;
}

/* ------------------------------------------------------------------------- */
/* Assertion 1: name_lookup                                                  */
/* ------------------------------------------------------------------------- */

static void assertion_name_lookup(void)
{
    static const char *const names[3] = {
        "nvnDeviceBuilderSetDefaults",
        "nvnCommandBufferSetDepthRange",
        "nvnSamplerGetLodBias",
    };
    void *boot_ptr[3];
    PFN_nvnDeviceGetProcAddress get_proc;

    assertion_begin("name_lookup");
    memset(boot_ptr, 0, sizeof boot_ptr);

    for (int i = 0; i < 3; i++) {
        void *again;

        boot_ptr[i] = boot(names[i]);
        CHECK_MSG(boot_ptr[i] != NULL, "bootstrap returned NULL for %s",
                  names[i]);
        again = nvnBootstrapLoader(names[i]);
        CHECK_MSG(again == boot_ptr[i],
                  "second bootstrap(\"%s\") returned a different VA (%p vs %p)",
                  names[i], again, boot_ptr[i]);
    }

    CHECK_MSG(boot_ptr[0] && boot_ptr[1] && boot_ptr[2],
              "one or more bootstrap lookups failed");
    CHECK_MSG(boot_ptr[0] != boot_ptr[1] && boot_ptr[0] != boot_ptr[2] &&
                  boot_ptr[1] != boot_ptr[2],
              "distinct names returned the same thunk (%p %p %p)", boot_ptr[0],
              boot_ptr[1], boot_ptr[2]);

    get_proc = (PFN_nvnDeviceGetProcAddress)boot("nvnDeviceGetProcAddress");
    CHECK(get_proc != NULL);

    /* The same g_dev object is initialized in assertion 3.  GetProcAddress is
     * a table lookup in the spike; it only needs a magic-bearing device. */
    for (int i = 0; i < 3; i++) {
        void *table_ptr = resolve_table(names[i]);
        void *proc_ptr = get_proc ? get_proc(&g_dev, names[i]) : NULL;

        CHECK_MSG(boot_ptr[i] && table_ptr && proc_ptr &&
                      boot_ptr[i] == table_ptr && boot_ptr[i] == proc_ptr,
                  "%s VA mismatch: boot=%p resolve=%p getproc=%p", names[i],
                  boot_ptr[i], table_ptr, proc_ptr);
    }

    {
        const char *unknown_name = "nvnThisNameDoesNotExist";
        uint64_t (*unknown_fn)(void) =
            (uint64_t (*)(void))boot(unknown_name);

        CHECK(unknown_fn != NULL);
        if (unknown_fn) {
            uint64_t r = unknown_fn();
            CHECK_MSG(r == 0, "unknown bootstrap stub returned 0x%016" PRIx64,
                      r);
        }

        {
            uint64_t (*table_unknown)(void) =
                (uint64_t (*)(void))resolve_table(unknown_name);
            CHECK(table_unknown != NULL);
            if (table_unknown) {
                uint64_t r = table_unknown();
                CHECK_MSG(r == 0,
                          "unknown table stub returned 0x%016" PRIx64, r);
            }
        }
    }

    {
        nva_spike_state st;
        if (get_state(&st)) {
            nva_nvn_resolve_result known_rr;
            nva_nvn_resolve_result unknown_rr;
            uint64_t probe_va = nva_nvn_resolve(g_table, "nvaSpikeTlsProbe");

            memset(&known_rr, 0, sizeof known_rr);
            memset(&unknown_rr, 0, sizeof unknown_rr);
            CHECK(nva_nvn_resolve_ex(g_table, "nvaSpikeTlsProbe", &known_rr) ==
                  NVA_OK);
            CHECK(known_rr.target == probe_va);
            CHECK(known_rr.slot == 12);
            CHECK((known_rr.flags & NVA_NVN_RESOLVE_KNOWN) != 0);
            CHECK(((known_rr.flags & NVA_NVN_RESOLVE_GATE) != 0) ==
                  (st.tls_gate != 0));

            CHECK(nva_nvn_resolve_ex(g_table, "nvnThisNameDoesNotExist",
                                     &unknown_rr) == NVA_OK);
            CHECK(unknown_rr.target != 0);
            CHECK(unknown_rr.slot == UINT32_MAX);
            CHECK((unknown_rr.flags & NVA_NVN_RESOLVE_UNKNOWN) != 0);

            CHECK_MSG(st.unknown_resolves >= 1,
                      "unknown_resolves=%" PRIu32 " (expected >= 1)",
                      st.unknown_resolves);
            (void)snprintf(g_extra, sizeof g_extra,
                           ",\"unknown_resolves\":%" PRIu32
                           ",\"resolve_ex_slot\":%" PRIu32
                           ",\"resolve_ex_gate\":%s",
                           st.unknown_resolves, known_rr.slot,
                           (known_rr.flags & NVA_NVN_RESOLVE_GATE) ? "true"
                                                                  : "false");
        }
    }

    (void)assertion_end();
}

/* ------------------------------------------------------------------------- */
/* Assertion 2: state_setter                                                 */
/* ------------------------------------------------------------------------- */

static void assertion_state_setter(void)
{
    PFN_nvnCommandBufferSetDepthRange set_range;
    NVNcommandBuffer cb;

    assertion_begin("state_setter");
    set_range = (PFN_nvnCommandBufferSetDepthRange)
        boot("nvnCommandBufferSetDepthRange");
    CHECK(set_range != NULL);

    memset(&cb, 0, sizeof cb);
    cb.magic = NVN_MAGIC_CMDBUF;
    if (set_range)
        set_range(&cb, 0.25f, 0.75f);

    {
        nva_spike_state st;
        if (get_state(&st)) {
            CHECK_MSG(st.depth_bits_near == 0x3e800000u,
                      "depth_bits_near=0x%08" PRIx32 " (expected 0x3e800000)",
                      st.depth_bits_near);
            CHECK_MSG(st.depth_bits_far == 0x3f400000u,
                      "depth_bits_far=0x%08" PRIx32 " (expected 0x3f400000)",
                      st.depth_bits_far);
            (void)snprintf(g_extra, sizeof g_extra,
                           ",\"depth_bits_near\":\"0x%08" PRIx32 "\""
                           ",\"depth_bits_far\":\"0x%08" PRIx32 "\"",
                           st.depth_bits_near, st.depth_bits_far);
        }
    }

    (void)assertion_end();
}

/* ------------------------------------------------------------------------- */
/* Assertion 3: draw_vulkan                                                  */
/* ------------------------------------------------------------------------- */

static void assertion_draw_vulkan(void)
{
    PFN_nvnDeviceBuilderSetDefaults set_defaults;
    PFN_nvnDeviceInitialize dev_init;
    PFN_nvnCommandBufferClearColor clear_color;
    PFN_nvnCommandBufferDrawArrays draw_arrays;
    PFN_nvnQueueSubmitCommands submit;
    PFN_nvnQueueFinish finish;
    NVNdeviceBuilder builder;
    NVNqueue q;
    NVNcommandBuffer cb;
    NVNcommandHandle handle;
    float red[4] = { 1.0f, 0.0f, 0.0f, 1.0f };
    int validation_unavailable = 0;

    assertion_begin("draw_vulkan");
    set_defaults = (PFN_nvnDeviceBuilderSetDefaults)
        boot("nvnDeviceBuilderSetDefaults");
    dev_init = (PFN_nvnDeviceInitialize)boot("nvnDeviceInitialize");
    clear_color = (PFN_nvnCommandBufferClearColor)
        boot("nvnCommandBufferClearColor");
    draw_arrays = (PFN_nvnCommandBufferDrawArrays)
        boot("nvnCommandBufferDrawArrays");
    submit = (PFN_nvnQueueSubmitCommands)boot("nvnQueueSubmitCommands");
    finish = (PFN_nvnQueueFinish)boot("nvnQueueFinish");

    CHECK(set_defaults != NULL);
    CHECK(dev_init != NULL);
    CHECK(clear_color != NULL);
    CHECK(draw_arrays != NULL);
    CHECK(submit != NULL);
    CHECK(finish != NULL);

    memset(&builder, 0, sizeof builder);
    if (set_defaults)
        set_defaults(&builder);

    if (dev_init) {
        uint32_t r = dev_init(&g_dev, &builder);
        CHECK_MSG(r == 1, "nvnDeviceInitialize returned %" PRIu32
                          " (expected 1)", r);
    }

    memset(&q, 0, sizeof q);
    q.magic = NVN_MAGIC_QUEUE;

    memset(&cb, 0, sizeof cb);
    cb.magic = NVN_MAGIC_CMDBUF;
    if (clear_color)
        clear_color(&cb, 0, red, 0xF);
    if (draw_arrays)
        draw_arrays(&cb, NVN_DRAW_TRIANGLES, 0, 3);

    handle = (NVNcommandHandle)&cb;
    if (submit)
        submit(&q, 1, &handle);
    if (finish)
        finish(&q);

    {
        nva_spike_state st;
        if (get_state(&st)) {
            uint32_t r = st.pixel_rgba & 0xffu;
            uint32_t g = (st.pixel_rgba >> 8) & 0xffu;
            uint32_t b = (st.pixel_rgba >> 16) & 0xffu;
            uint32_t a = (st.pixel_rgba >> 24) & 0xffu;
            char backend[65];
            char device[129];
            char backend_esc[420];
            char device_esc[820];
            const char *validation;

            CHECK_MSG(st.fence_wait_ok == 1,
                      "fence_wait_ok=%" PRIu32 " (expected 1)",
                      st.fence_wait_ok);
            CHECK_MSG(st.last_serial >= 1,
                      "last_serial=%" PRIu32 " (expected >= 1)",
                      st.last_serial);
            CHECK_MSG(g >= 200u && r <= 50u && b <= 50u,
                      "center pixel 0x%08" PRIx32
                      " r=%" PRIu32 " g=%" PRIu32 " b=%" PRIu32
                      " a=%" PRIu32, st.pixel_rgba, r, g, b, a);

            if (st.validation_available == 1) {
                validation = "ok";
                CHECK_MSG(st.validation_errors == 0,
                          "validation_errors=%" PRIu32,
                          st.validation_errors);
            } else {
                validation = "unavailable";
                validation_unavailable = 1;
            }

            copy_cstr(backend, sizeof backend, st.backend,
                      sizeof st.backend);
            copy_cstr(device, sizeof device, st.device, sizeof st.device);
            json_escape(backend_esc, sizeof backend_esc, backend);
            json_escape(device_esc, sizeof device_esc, device);
            (void)snprintf(g_extra, sizeof g_extra,
                           ",\"backend\":\"%s\",\"device\":\"%s\""
                           ",\"pixel_rgba\":\"0x%08" PRIx32 "\""
                           ",\"validation\":\"%s\"",
                           backend_esc, device_esc, st.pixel_rgba, validation);
        }
    }
    (void)validation_unavailable;

    (void)assertion_end();
}

/* ------------------------------------------------------------------------- */
/* Assertion 4: returns                                                      */
/* ------------------------------------------------------------------------- */

/* Direct-symbol struct-return contract (spike-only extras).  Kept as named
 * constants so the host and guest agree; a mismatch is reported via pair_ok /
 * triple_ok but is not part of the five required assertions. */
#define SPIKE_PAIR_A 0x1122334455667788ULL
#define SPIKE_PAIR_B 0x99aabbccddeeff00ULL
#define SPIKE_TRIPLE_A 0x0102030405060708ULL
#define SPIKE_TRIPLE_B 0x1112131415161718ULL
#define SPIKE_TRIPLE_C 0x2122232425262728ULL

static void assertion_returns(void)
{
    PFN_nvnSamplerGetLodBias get_lod;
    PFN_nvnBufferGetAddress get_address;
    NVNsampler sampler;
    NVNbuffer buffer;
    float lod;
    uint32_t lod_bits = 0;
    uint64_t address = 0;
    nvaSpikePair pair;
    nvaSpikeTriple triple;
    int pair_ok;
    int triple_ok;

    assertion_begin("returns");
    get_lod = (PFN_nvnSamplerGetLodBias)boot("nvnSamplerGetLodBias");
    get_address = (PFN_nvnBufferGetAddress)boot("nvnBufferGetAddress");
    CHECK(get_lod != NULL);
    CHECK(get_address != NULL);

    memset(&sampler, 0, sizeof sampler);
    sampler.magic = NVN_MAGIC_SAMPLER;
    sampler.lod_bias = 1.25f;
    if (get_lod) {
        lod = get_lod(&sampler);
        memcpy(&lod_bits, &lod, sizeof lod_bits);
    }
    CHECK_MSG(lod_bits == 0x3fa00000u,
              "SamplerGetLodBias bits=0x%08" PRIx32
              " (expected 0x3fa00000)", lod_bits);

    memset(&buffer, 0, sizeof buffer);
    buffer.magic = NVN_MAGIC_BUFFER;
    buffer.address = 0x123456789abcdef0ULL;
    if (get_address)
        address = get_address(&buffer);
    CHECK_MSG(address == 0x123456789abcdef0ULL,
              "BufferGetAddress=0x%016" PRIx64
              " (expected 0x123456789abcdef0)", address);

    /* Extras: direct X0/X1 and X8 struct returns.  Informational only. */
    pair = nvaSpikeReturnPair();
    triple = nvaSpikeReturnTriple();
    pair_ok = (pair.a == SPIKE_PAIR_A && pair.b == SPIKE_PAIR_B);
    triple_ok = (triple.a == SPIKE_TRIPLE_A && triple.b == SPIKE_TRIPLE_B &&
                 triple.c == SPIKE_TRIPLE_C);

    (void)snprintf(g_extra, sizeof g_extra,
                   ",\"lod_bits\":\"0x%08" PRIx32 "\""
                   ",\"buffer_address\":\"0x%016" PRIx64 "\""
                   ",\"pair_a\":\"0x%016" PRIx64 "\""
                   ",\"pair_b\":\"0x%016" PRIx64 "\""
                   ",\"pair_ok\":%s"
                   ",\"triple_a\":\"0x%016" PRIx64 "\""
                   ",\"triple_b\":\"0x%016" PRIx64 "\""
                   ",\"triple_c\":\"0x%016" PRIx64 "\""
                   ",\"triple_ok\":%s",
                   lod_bits, address, pair.a, pair.b,
                   pair_ok ? "true" : "false", triple.a, triple.b, triple.c,
                   triple_ok ? "true" : "false");

    (void)assertion_end();
}

/* ------------------------------------------------------------------------- */
/* Assertion 5: unimplemented_typed                                          */
/* ------------------------------------------------------------------------- */

static void assertion_unimplemented_typed(void)
{
    PFN_nvnDeviceGetCurrentTimestampInNanoseconds get_ts;
    uint32_t before = 0;
    uint64_t ts = 1;

    assertion_begin("unimplemented_typed");
    get_ts = (PFN_nvnDeviceGetCurrentTimestampInNanoseconds)
        boot("nvnDeviceGetCurrentTimestampInNanoseconds");
    CHECK(get_ts != NULL);

    {
        nva_spike_state st;
        if (get_state(&st))
            before = st.unsupported_calls;
    }

    if (get_ts)
        ts = get_ts(&g_dev);
    CHECK_MSG(ts == 0, "timestamp stub returned 0x%016" PRIx64
                       " (expected typed zero)", ts);

    {
        nva_spike_state st;
        if (get_state(&st)) {
            CHECK_MSG(st.unsupported_calls >= 1,
                      "unsupported_calls=%" PRIu32 " (expected >= 1)",
                      st.unsupported_calls);
            (void)snprintf(g_extra, sizeof g_extra,
                           ",\"timestamp\":\"0x%016" PRIx64 "\""
                           ",\"unsupported_before\":%" PRIu32
                           ",\"unsupported_after\":%" PRIu32,
                           ts, before, st.unsupported_calls);
        }
    }

    (void)assertion_end();
}

static void run_five_assertions(void)
{
    assertion_name_lookup();
    assertion_state_setter();
    assertion_draw_vulkan();
    assertion_returns();
    assertion_unimplemented_typed();
}

/* ------------------------------------------------------------------------- */
/* AArch64-only probes: TPIDR_EL0, x18, FPCR, stack arguments                 */
/* ------------------------------------------------------------------------- */

#if defined(__aarch64__)

#define NVN_FAKE_TLS_SIZE 4096u
#define NVN_X18_SENTINEL 0x18badc0deULL

#if defined(__ANDROID__) || (defined(NVN_X18_RESERVED) && NVN_X18_RESERVED)
#define NVN_X18_REQUIRED 1
#else
#define NVN_X18_REQUIRED 0
#endif

/* TPIDR_EL0 captured by the host at nva_gfx_create() on this (main) thread. */
static uint64_t g_host_tls;

static uint64_t read_tpidr_el0(void)
{
    uint64_t v = 0;
    __asm__ volatile("mrs %0, tpidr_el0" : "=r"(v) : : "memory");
    return v;
}

static void write_tpidr_el0(uint64_t v)
{
    __asm__ volatile("msr tpidr_el0, %0" : : "r"(v) : "memory");
}

static void set_x18(uint64_t v)
{
    __asm__ volatile("mov x18, %0" : : "r"(v) : "memory");
}

static uint64_t get_x18(void)
{
    uint64_t v = 0;
    __asm__ volatile("mov %0, x18" : "=r"(v) : : "memory");
    return v;
}

static uint64_t read_fpcr(void)
{
    uint64_t v = 0;
    __asm__ volatile("mrs %0, fpcr" : "=r"(v) : : "memory");
    return v;
}

static void write_fpcr(uint64_t v)
{
    __asm__ volatile("msr fpcr, %0" : : "r"(v) : "memory");
}

/* Installs a fake TPIDR_EL0, calls the thunk, reads TPIDR_EL0 back, and
 * restores the real TLS.  The only code between install and restore is the
 * thunk call and inline asm: no libc, no printf, no stack-protector TLS. */
static uint64_t call_with_fake_tls(PFN_nvaSpikeTlsProbe fn, void *fake,
                                   uint64_t *out_after)
{
    uint64_t saved = read_tpidr_el0();
    uint64_t fake_va = (uint64_t)(uintptr_t)fake;
    uint64_t probe;
    uint64_t after;

    write_tpidr_el0(fake_va);
    g_fake_tls_active = 1;
    probe = fn();
    after = read_tpidr_el0();
    g_fake_tls_active = 0;
    write_tpidr_el0(saved);

    if (out_after)
        *out_after = after;
    return probe;
}

static void probe_tls_direct(void)
{
    PFN_nvaSpikeTlsProbe fn;
    nva_spike_state st;
    void *fake;

    assertion_begin("tls_direct_hazard");

    /* In gate mode the table currently hands out gate thunks; switch back to
     * the direct shape before measuring the hazard. */
    if (get_state(&st) && st.tls_gate != 0) {
        nva_status s = nva_spike_set_tls_gate(g_gfx, 0);
        CHECK_MSG(s == NVA_OK,
                  "nva_spike_set_tls_gate(0) -> %" PRIu32, s);
    }

    fn = (PFN_nvaSpikeTlsProbe)(uintptr_t)
        nva_nvn_resolve(g_table, "nvaSpikeTlsProbe");
    CHECK(fn != NULL);

    if (posix_memalign(&fake, NVN_FAKE_TLS_SIZE, NVN_FAKE_TLS_SIZE) != 0)
        fake = NULL;
    CHECK_MSG(fake != NULL, "posix_memalign(%u,%u) failed",
              NVN_FAKE_TLS_SIZE, NVN_FAKE_TLS_SIZE);
    if (fn && fake) {
        uint64_t fake_va = (uint64_t)(uintptr_t)fake;
        uint64_t guest_tls = read_tpidr_el0();
        uint64_t after = 0;
        uint64_t probe;

        memset(fake, 0x5a, NVN_FAKE_TLS_SIZE);
        probe = call_with_fake_tls(fn, fake, &after);

        CHECK_MSG(probe == fake_va,
                  "direct thunk saw 0x%016" PRIx64
                  ", expected fake TLS 0x%016" PRIx64, probe, fake_va);
        CHECK_MSG(probe != g_host_tls,
                  "direct thunk unexpectedly saw host TLS 0x%016" PRIx64,
                  probe);
        CHECK_MSG(after == fake_va,
                  "TPIDR_EL0 after direct thunk is 0x%016" PRIx64
                  " (expected fake 0x%016" PRIx64 ")", after, fake_va);
        (void)snprintf(g_extra, sizeof g_extra,
                       ",\"informational\":true"
                       ",\"probe\":\"0x%016" PRIx64 "\""
                       ",\"fake\":\"0x%016" PRIx64 "\""
                       ",\"host_tls\":\"0x%016" PRIx64 "\""
                       ",\"guest_tls\":\"0x%016" PRIx64 "\""
                       ",\"tls_after\":\"0x%016" PRIx64 "\"",
                       probe, fake_va, g_host_tls, guest_tls, after);
    }
    free(fake);

    (void)assertion_end();
}

static void probe_x18(void)
{
    PFN_nvaSpikeTlsProbe fn;

    assertion_begin("x18_preserved");
    fn = (PFN_nvaSpikeTlsProbe)(uintptr_t)
        nva_nvn_resolve(g_table, "nvaSpikeTlsProbe");
    CHECK(fn != NULL);

    if (fn) {
        uint64_t after;
        int x18_ok;

        set_x18(NVN_X18_SENTINEL);
        (void)fn();
        after = get_x18();
        x18_ok = (after == NVN_X18_SENTINEL);

#if NVN_X18_REQUIRED
        CHECK_MSG(x18_ok,
                  "x18 changed across the thunk: 0x%016" PRIx64
                  " -> 0x%016" PRIx64, (uint64_t)NVN_X18_SENTINEL, after);
#endif
        (void)snprintf(g_extra, sizeof g_extra,
                       ",\"x18_before\":\"0x%016" PRIx64 "\""
                       ",\"x18_after\":\"0x%016" PRIx64 "\""
                       ",\"x18_ok\":%s,\"required\":%s",
                       (uint64_t)NVN_X18_SENTINEL, after,
                       x18_ok ? "true" : "false",
                       NVN_X18_REQUIRED ? "true" : "false");
    }

    (void)assertion_end();
}

static void probe_fpcr(void)
{
    PFN_nvnSamplerGetLodBias get_lod;
    const uint64_t rmode_mask = 3ull << 22;

    assertion_begin("fpcr_preserved");
    get_lod = (PFN_nvnSamplerGetLodBias)(uintptr_t)
        nva_nvn_resolve(g_table, "nvnSamplerGetLodBias");
    CHECK(get_lod != NULL);

    if (get_lod) {
        NVNsampler sampler;
        uint64_t initial = read_fpcr();
        uint64_t toggled = (initial & ~rmode_mask) | rmode_mask;
        uint64_t after;
        float lod;

        memset(&sampler, 0, sizeof sampler);
        sampler.magic = NVN_MAGIC_SAMPLER;
        sampler.lod_bias = 1.25f;

        write_fpcr(toggled);
        lod = get_lod(&sampler);
        after = read_fpcr();
        write_fpcr(initial);
        (void)lod;

        CHECK_MSG(after == toggled,
                  "FPCR changed across float return: 0x%016" PRIx64
                  " -> 0x%016" PRIx64, toggled, after);
        (void)snprintf(g_extra, sizeof g_extra,
                       ",\"initial\":\"0x%016" PRIx64 "\""
                       ",\"toggled\":\"0x%016" PRIx64 "\""
                       ",\"after\":\"0x%016" PRIx64 "\""
                       ",\"rmode_preserved\":%s",
                       initial, toggled, after,
                       (after & rmode_mask) == rmode_mask ? "true" : "false");
    }

    (void)assertion_end();
}

static void probe_stack_direct(void)
{
    PFN_nvaSpikeStackProbe fn;
    uint64_t sum = 0;

    assertion_begin("stack_args_direct");
    fn = (PFN_nvaSpikeStackProbe)(uintptr_t)
        nva_nvn_resolve(g_table, "nvaSpikeStackProbe");
    CHECK(fn != NULL);
    if (fn)
        sum = fn(1, 2, 3, 4, 5, 6, 7, 8, 9, 10);
    CHECK_MSG(sum == 55, "stack probe sum=%" PRIu64 " (expected 55)", sum);
    (void)snprintf(g_extra, sizeof g_extra, ",\"sum\":%" PRIu64, sum);
    (void)assertion_end();
}

static void probe_tls_gate(void)
{
    PFN_nvaSpikeTlsProbe fn;
    nva_spike_state st;
    nva_status s;
    void *fake;

    assertion_begin("tls_gate");

    s = nva_spike_set_tls_gate(g_gfx, 1);
    CHECK_MSG(s == NVA_OK, "nva_spike_set_tls_gate(1) -> %" PRIu32, s);

    fn = (PFN_nvaSpikeTlsProbe)(uintptr_t)
        nva_nvn_resolve(g_table, "nvaSpikeTlsProbe");
    CHECK(fn != NULL);

    if (get_state(&st)) {
        g_host_tls = st.host_tls;
        CHECK_MSG(st.tls_gate == 1,
                  "state.tls_gate=%" PRIu32 " after enabling the gate",
                  st.tls_gate);
        CHECK_MSG(st.host_tls != 0, "state.host_tls is 0");
    }

    if (posix_memalign(&fake, NVN_FAKE_TLS_SIZE, NVN_FAKE_TLS_SIZE) != 0)
        fake = NULL;
    CHECK_MSG(fake != NULL, "posix_memalign(%u,%u) failed",
              NVN_FAKE_TLS_SIZE, NVN_FAKE_TLS_SIZE);
    if (fn && fake && g_host_tls != 0) {
        uint64_t fake_va = (uint64_t)(uintptr_t)fake;
        uint64_t after = 0;
        uint64_t probe;

        memset(fake, 0x5a, NVN_FAKE_TLS_SIZE);
        probe = call_with_fake_tls(fn, fake, &after);

        CHECK_MSG(probe == g_host_tls,
                  "gate thunk saw 0x%016" PRIx64
                  ", expected host TLS 0x%016" PRIx64, probe, g_host_tls);
        CHECK_MSG(probe != fake_va,
                  "gate thunk saw the fake guest TLS 0x%016" PRIx64, probe);
        CHECK_MSG(after == fake_va,
                  "gate did not restore guest TLS: TPIDR_EL0=0x%016" PRIx64
                  " (expected fake 0x%016" PRIx64 ")", after, fake_va);
        (void)snprintf(g_extra, sizeof g_extra,
                       ",\"probe\":\"0x%016" PRIx64 "\""
                       ",\"host_tls\":\"0x%016" PRIx64 "\""
                       ",\"fake\":\"0x%016" PRIx64 "\""
                       ",\"tls_after\":\"0x%016" PRIx64 "\"",
                       probe, g_host_tls, fake_va, after);
    }
    free(fake);

    (void)assertion_end();
}

static void run_aarch64_probes(int gate_mode)
{
    probe_tls_direct();
    probe_x18();
    probe_fpcr();
    probe_stack_direct();
    if (gate_mode)
        probe_tls_gate();
}

#endif /* __aarch64__ */

/* ------------------------------------------------------------------------- */
/* Modes                                                                     */
/* ------------------------------------------------------------------------- */

static int run_unimplemented_mode(void)
{
    PFN_nvnCommandBufferDispatchCompute dispatch;
    NVNcommandBuffer cb;

    dispatch = (PFN_nvnCommandBufferDispatchCompute)
        boot("nvnCommandBufferDispatchCompute");
    memset(&cb, 0, sizeof cb);
    cb.magic = NVN_MAGIC_CMDBUF;

    if (!dispatch) {
        printf("RESULT {\"assertion\":\"unimplemented_abort\","
               "\"armed\":false,\"ok\":false,"
               "\"detail\":\"nvnCommandBufferDispatchCompute did not resolve\"}\n");
        fflush(NULL);
        return 1;
    }

    printf("RESULT {\"assertion\":\"unimplemented_abort\",\"armed\":true}\n");
    fflush(NULL);

    /* Deliberately not caught: the parent expects SIGABRT (exit 134). */
    dispatch(&cb, 1, 1, 1);

    printf("RESULT {\"assertion\":\"unimplemented_abort\",\"armed\":true,"
           "\"ok\":false,\"detail\":\"abort stub returned normally\"}\n");
    fflush(NULL);
    return 1;
}

static int run_stackgate_mode(void)
{
    const char *mode = "stackgate";

#if defined(__aarch64__)
    PFN_nvaSpikeStackProbe fn;
    uint64_t sum = 0;
    nva_status s;
    int ok;

    assertion_begin("stack_args_gate");
    s = nva_spike_set_tls_gate(g_gfx, 1);
    CHECK_MSG(s == NVA_OK, "nva_spike_set_tls_gate(1) -> %" PRIu32, s);
    fn = (PFN_nvaSpikeStackProbe)(uintptr_t)
        nva_nvn_resolve(g_table, "nvaSpikeStackProbe");
    CHECK(fn != NULL);
    if (fn)
        sum = fn(1, 2, 3, 4, 5, 6, 7, 8, 9, 10);

    CHECK_MSG(sum != 55,
              "gate stack probe returned 55: the gate did not shift SP "
              "(sum=%" PRIu64 ")", sum);
    (void)snprintf(g_extra, sizeof g_extra,
                   ",\"expected_broken\":true,\"sum\":%" PRIu64
                   ",\"expected_sum\":55", sum);
    ok = assertion_end();
    emit_summary(mode);
    return ok ? 0 : 1;
#else
    printf("SKIP stackgate\n");
    fflush(stdout);
    emit_summary(mode);
    return 0;
#endif
}

int main(int argc, char **argv)
{
    const char *mode = (argc > 1) ? argv[1] : "all";
    int is_all = (strcmp(mode, "all") == 0);
    int is_gate = (strcmp(mode, "gate") == 0);
    int is_unimplemented = (strcmp(mode, "unimplemented") == 0);
    int is_stackgate = (strcmp(mode, "stackgate") == 0);

    if (!is_all && !is_gate && !is_unimplemented && !is_stackgate) {
        fprintf(stderr, "usage: %s [all|gate|unimplemented|stackgate]\n",
                argv[0]);
        return 2;
    }

    if (!guest_init()) {
        emit_summary(mode);
        return 1;
    }

    /* The same object answers GetProcAddress in assertion 1 and is
     * initialized by assertion 3. */
    memset(&g_dev, 0, sizeof g_dev);
    g_dev.magic = NVN_MAGIC_DEVICE;

#if defined(__aarch64__)
    {
        nva_spike_state st;
        if (query_state(&st) != NVA_OK) {
            assertion_begin("init");
            failf("nva_spike_get_state failed immediately after create");
            (void)assertion_end();
            emit_summary(mode);
            return 1;
        }
        /* Captured on this thread by nva_gfx_create(); the exec runs on the
         * main thread, so it is also the guest's normal TPIDR_EL0. */
        g_host_tls = st.host_tls;
    }
#endif

    if (is_unimplemented)
        return run_unimplemented_mode();

    if (is_stackgate)
        return run_stackgate_mode();

#if !defined(__aarch64__)
    if (is_gate) {
        printf("SKIP tls_gate\n");
        fflush(stdout);
    }
#else
    if (is_gate) {
        nva_status s = nva_spike_set_tls_gate(g_gfx, 1);
        if (s != NVA_OK) {
            assertion_begin("init");
            failf("nva_spike_set_tls_gate(1) -> %" PRIu32, s);
            (void)assertion_end();
            emit_summary(mode);
            return 1;
        }
    }
#endif

    run_five_assertions();

#if defined(__aarch64__)
    run_aarch64_probes(is_gate);
#endif

    emit_summary(mode);
    return (g_failures == 0) ? 0 : 1;
}
