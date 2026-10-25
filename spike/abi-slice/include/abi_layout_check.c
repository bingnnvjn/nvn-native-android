/* Compile-time C-side layout checks for the v1 structs used by the spike.
 * Build with: cc -std=c11 -Iinclude -c include/abi_layout_check.c
 */
#include "nva_abi.h"
#include "nvn_guest.h"

_Static_assert(sizeof(nva_host_services) == 48, "nva_host_services size");
_Static_assert(sizeof(nva_gfx_desc) == 24, "nva_gfx_desc size");
_Static_assert(sizeof(nva_nvn_slot) == 16, "nva_nvn_slot size");
_Static_assert(sizeof(nva_nvn_table) == 48, "nva_nvn_table size");
_Static_assert(sizeof(nva_mem_region) == 104, "nva_mem_region size");
_Static_assert(offsetof(nva_mem_region, guest_va) == 8, "region.guest_va");
_Static_assert(offsetof(nva_mem_region, alloc_handle) == 40, "region.alloc_handle");
_Static_assert(offsetof(nva_mem_region, cpu_ptr) == 48, "region.cpu_ptr");
_Static_assert(offsetof(nva_mem_region, gpu_offset) == 64, "region.gpu_offset");
_Static_assert(offsetof(nva_mem_region, alias_of) == 72, "region.alias_of");
_Static_assert(offsetof(nva_mem_region, region_id) == 88, "region.region_id");
_Static_assert(offsetof(nva_mem_region, epoch) == 96, "region.epoch");

_Static_assert(sizeof(NVNop) == 32, "NVNop size");
_Static_assert(sizeof(NVNcommandBuffer) == 544, "NVNcommandBuffer size");
_Static_assert(offsetof(NVNcommandBuffer, ops) == 28, "NVNcommandBuffer.ops");
_Static_assert(sizeof(NVNsampler) == 24, "NVNsampler size");
_Static_assert(offsetof(NVNsampler, lod_bias) == 8, "NVNsampler.lod_bias");
_Static_assert(sizeof(NVNbuffer) == 16, "NVNbuffer size");
_Static_assert(sizeof(NVNdevice) == 64, "NVNdevice size");
_Static_assert(sizeof(nva_spike_state) == 392, "nva_spike_state size");
