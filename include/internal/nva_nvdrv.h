/*
 * nva_nvdrv.h — 内部 ABI（host → engine）：nvdrv 的 typed 接缝。
 *
 * 不在 v1 公开面；host 与 engine 同仓演进，v1.1 再决定是否安装。
 * guest 永不 include 本头：guest 的 /dev/nvhost-* ioctl 由 host 的 hle_nvdrv 服务
 * 校验 guest 指针/size 后，翻成本头只收 host 指针的 typed 调用。
 */
#pragma once

#include "../nva_abi.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct nva_nvdrv nva_nvdrv;
typedef uint64_t nva_nvdrv_handle;   /* index | gen<<32 */

nva_status nva_nvdrv_create(nva_gfx *gfx, nva_nvdrv **out);
void       nva_nvdrv_destroy(nva_nvdrv *nvdrv);

nva_status nva_nvdrv_open(nva_nvdrv *nvdrv, const char *path, uint32_t flags,
                          nva_nvdrv_handle *out);
nva_status nva_nvdrv_close(nva_nvdrv *nvdrv, nva_nvdrv_handle handle);

/* 逃生口：只收 host 指针，不收 guest VA；未知 cmd 返回 UNSUPPORTED，不转发。 */
nva_status nva_nvdrv_ioctl(nva_nvdrv *nvdrv, nva_nvdrv_handle handle, uint32_t cmd,
                           const void *in, uint64_t in_size,
                           void *out, uint64_t out_size, uint64_t *out_count);

nva_status nva_nvdrv_channel_create(nva_nvdrv *nvdrv, uint32_t flags,
                                    nva_nvdrv_handle *out);
nva_status nva_nvdrv_channel_destroy(nva_nvdrv *nvdrv, nva_nvdrv_handle channel);

/* COPY 默认；ALIAS 需显式 flag + 能力探测（VK_EXT_external_memory_host 在 725 未验证）。 */
#define NVA_NVMAP_COPY  0u
#define NVA_NVMAP_ALIAS 1u
nva_status nva_nvdrv_nvmap_create(nva_nvdrv *nvdrv, uint64_t size, uint64_t align,
                                  uint32_t flags, nva_nvdrv_handle *out);
nva_status nva_nvdrv_nvmap_destroy(nva_nvdrv *nvdrv, nva_nvdrv_handle nvmap);

nva_status nva_nvdrv_syncpoint_create(nva_nvdrv *nvdrv, nva_nvdrv_handle *out);
nva_status nva_nvdrv_syncpoint_wait(nva_nvdrv *nvdrv, nva_nvdrv_handle syncpoint,
                                    uint32_t threshold, uint64_t timeout_ns);
nva_status nva_nvdrv_syncpoint_signal(nva_nvdrv *nvdrv, nva_nvdrv_handle syncpoint,
                                      uint32_t value);

/* 设备丢失：engine 置位并唤醒所有等待；host 收到后 gen++、作废句柄。 */
nva_status nva_nvdrv_on_device_lost(nva_nvdrv *nvdrv, uint32_t reason);

#ifdef __cplusplus
}
#endif
