//! Offscreen Vulkan 1.1 backend for the #30 ABI spike.
//!
//! One 64x64 R8G8B8A8_UNORM image, a classic render pass, a fullscreen-triangle
//! pipeline (no vertex input), a fence, and a host-visible readback buffer.
//! No WSI, no swapchain, no Android surface: this works on lavapipe and on an
//! Adreno 725 from an `adb shell` executable.

use std::ffi::{c_char, c_void, CStr, CString};
use std::io::Cursor;
use std::slice;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use ash::ext::debug_utils;
use ash::vk;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;

#[derive(Clone, Debug)]
pub struct VkInfo {
    pub backend: String,
    pub device: String,
    pub validation: bool,
}

pub struct Vk {
    _entry: ash::Entry,
    instance: ash::Instance,
    device: ash::Device,
    queue: vk::Queue,
    physical: vk::PhysicalDevice,

    image: vk::Image,
    image_memory: vk::DeviceMemory,
    image_view: vk::ImageView,
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    vert_module: vk::ShaderModule,
    frag_module: vk::ShaderModule,

    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,

    readback_buffer: vk::Buffer,
    readback_memory: vk::DeviceMemory,

    debug_utils: Option<debug_utils::Instance>,
    debug_messenger: Option<vk::DebugUtilsMessengerEXT>,
    validation_errors: Arc<AtomicU32>,
    validation: bool,

    info: VkInfo,
    serial: u32,
    pixel: [u8; 4],
}

impl Vk {
    pub fn new(enable_validation: bool) -> Result<Self, String> {
        unsafe { Self::new_unsafe(enable_validation) }
    }

    unsafe fn new_unsafe(enable_validation: bool) -> Result<Self, String> {
        let entry = ash::Entry::load().map_err(|e| format!("load libvulkan: {e}"))?;

        let layer_name = CString::new("VK_LAYER_KHRONOS_validation").unwrap();
        let debug_ext_name = CString::new("VK_EXT_debug_utils").unwrap();
        let use_validation = enable_validation && has_layer(&entry, &layer_name);
        let use_debug = use_validation && has_instance_ext(&entry, &debug_ext_name);

        let app_name = CString::new("abi-slice").unwrap();
        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(1)
            .engine_name(&app_name)
            .engine_version(1)
            .api_version(vk::make_api_version(0, 1, 1, 0));

        let layer_ptrs: Vec<*const c_char> = if use_validation {
            vec![layer_name.as_ptr()]
        } else {
            vec![]
        };
        let ext_ptrs: Vec<*const c_char> = if use_debug {
            vec![debug_ext_name.as_ptr()]
        } else {
            vec![]
        };
        let create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_layer_names(&layer_ptrs)
            .enabled_extension_names(&ext_ptrs);

        let instance = entry
            .create_instance(&create_info, None)
            .map_err(|e| format!("vkCreateInstance: {e}"))?;

        let errors = Arc::new(AtomicU32::new(0));
        let debug_utils = if use_debug {
            Some(debug_utils::Instance::new(&entry, &instance))
        } else {
            None
        };
        let debug_messenger = if let Some(du) = debug_utils.as_ref() {
            let cb_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
                .message_severity(
                    vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                        | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING,
                )
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                        | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                        | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                )
                .pfn_user_callback(Some(debug_callback))
                .user_data(Arc::as_ptr(&errors) as *mut c_void);
            match du.create_debug_utils_messenger(&cb_info, None) {
                Ok(m) => Some(m),
                Err(e) => {
                    eprintln!("[vk] debug messenger unavailable: {e}");
                    None
                }
            }
        } else {
            None
        };

        let devices = instance
            .enumerate_physical_devices()
            .map_err(|e| format!("vkEnumeratePhysicalDevices: {e}"))?;
        if devices.is_empty() {
            return Err("no Vulkan physical device".into());
        }

        let mut chosen = None;
        for pd in devices {
            let qfamilies = instance.get_physical_device_queue_family_properties(pd);
            if let Some((family, _)) = qfamilies
                .iter()
                .enumerate()
                .find(|(_, q)| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            {
                chosen = Some((pd, family as u32));
                break;
            }
        }
        let (physical, queue_family) =
            chosen.ok_or_else(|| "no Vulkan device with a graphics queue".to_string())?;

        let props = instance.get_physical_device_properties(physical);
        let device_name = cstr_array_to_string(&props.device_name);
        let version = props.api_version;
        let backend = format!(
            "vulkan {}.{}.{}",
            vk::api_version_major(version),
            vk::api_version_minor(version),
            vk::api_version_patch(version)
        );

        let priorities = [1.0f32];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priorities);
        let device = instance
            .create_device(
                physical,
                &vk::DeviceCreateInfo::default().queue_create_infos(slice::from_ref(&queue_info)),
                None,
            )
            .map_err(|e| format!("vkCreateDevice: {e}"))?;
        let queue = device.get_device_queue(queue_family, 0);

        let mem_props = instance.get_physical_device_memory_properties(physical);

        let extent = vk::Extent2D {
            width: WIDTH,
            height: HEIGHT,
        };

        // ---- image -----------------------------------------------------
        let image = device
            .create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(FORMAT)
                    .extent(vk::Extent3D {
                        width: WIDTH,
                        height: HEIGHT,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )
            .map_err(|e| format!("vkCreateImage: {e}"))?;
        let req = device.get_image_memory_requirements(image);
        let image_mem_type = find_memory_type(
            &mem_props,
            req.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )
        .ok_or_else(|| "no DEVICE_LOCAL memory type for image".to_string())?;
        let image_memory = device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(image_mem_type),
                None,
            )
            .map_err(|e| format!("allocate image memory: {e}"))?;
        device
            .bind_image_memory(image, image_memory, 0)
            .map_err(|e| format!("vkBindImageMemory: {e}"))?;

        let image_view = device
            .create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(FORMAT)
                    .components(vk::ComponentMapping::default())
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .base_mip_level(0)
                            .level_count(1)
                            .base_array_layer(0)
                            .layer_count(1),
                    ),
                None,
            )
            .map_err(|e| format!("vkCreateImageView: {e}"))?;

        // ---- render pass / framebuffer ---------------------------------
        let attachment = vk::AttachmentDescription::default()
            .format(FORMAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
        let color_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(slice::from_ref(&color_ref));
        let render_pass = device
            .create_render_pass(
                &vk::RenderPassCreateInfo::default()
                    .attachments(slice::from_ref(&attachment))
                    .subpasses(slice::from_ref(&subpass)),
                None,
            )
            .map_err(|e| format!("vkCreateRenderPass: {e}"))?;
        let framebuffer = device
            .create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(render_pass)
                    .attachments(slice::from_ref(&image_view))
                    .width(WIDTH)
                    .height(HEIGHT)
                    .layers(1),
                None,
            )
            .map_err(|e| format!("vkCreateFramebuffer: {e}"))?;

        // ---- pipeline ---------------------------------------------------
        let vert_spv = read_spv(include_bytes!(concat!(env!("OUT_DIR"), "/tri.vert.spv")))?;
        let frag_spv = read_spv(include_bytes!(concat!(env!("OUT_DIR"), "/tri.frag.spv")))?;
        let vert_module = device
            .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&vert_spv), None)
            .map_err(|e| format!("create vertex shader: {e}"))?;
        let frag_module = device
            .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&frag_spv), None)
            .map_err(|e| format!("create fragment shader: {e}"))?;
        let main = CString::new("main").unwrap();
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vert_module)
                .name(&main),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(frag_module)
                .name(&main),
        ];

        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(WIDTH as f32)
            .height(HEIGHT as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let scissor = vk::Rect2D::default().offset(vk::Offset2D { x: 0, y: 0 }).extent(extent);
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(slice::from_ref(&viewport))
            .scissors(slice::from_ref(&scissor));
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(false)
            .color_write_mask(vk::ColorComponentFlags::RGBA);
        let color_blend = vk::PipelineColorBlendStateCreateInfo::default()
            .attachments(slice::from_ref(&blend_attachment));
        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default();

        let pipeline_layout = device
            .create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default(), None)
            .map_err(|e| format!("vkCreatePipelineLayout: {e}"))?;
        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blend)
            .layout(pipeline_layout)
            .render_pass(render_pass)
            .subpass(0);
        let pipelines = device
            .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
            .map_err(|(_, e)| format!("vkCreateGraphicsPipelines: {e}"))?;
        let pipeline = pipelines[0];

        // ---- command pool / fence / readback ---------------------------
        let command_pool = device
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
            .map_err(|e| format!("vkCreateCommandPool: {e}"))?;
        let command_buffer = device
            .allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
            .map_err(|e| format!("vkAllocateCommandBuffers: {e}"))?[0];
        let fence = device
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .map_err(|e| format!("vkCreateFence: {e}"))?;

        let readback_size = (WIDTH * HEIGHT * 4) as vk::DeviceSize;
        let readback_buffer = device
            .create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(readback_size)
                    .usage(vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
            .map_err(|e| format!("vkCreateBuffer(readback): {e}"))?;
        let req = device.get_buffer_memory_requirements(readback_buffer);
        let readback_type = find_memory_type(
            &mem_props,
            req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )
        .ok_or_else(|| "no HOST_VISIBLE|HOST_COHERENT memory for readback".to_string())?;
        let readback_memory = device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(readback_type),
                None,
            )
            .map_err(|e| format!("allocate readback memory: {e}"))?;
        device
            .bind_buffer_memory(readback_buffer, readback_memory, 0)
            .map_err(|e| format!("vkBindBufferMemory: {e}"))?;

        Ok(Self {
            _entry: entry,
            instance,
            device,
            queue,
            physical,
            image,
            image_memory,
            image_view,
            render_pass,
            framebuffer,
            pipeline_layout,
            pipeline,
            vert_module,
            frag_module,
            command_pool,
            command_buffer,
            fence,
            readback_buffer,
            readback_memory,
            debug_utils,
            debug_messenger,
            validation_errors: errors,
            validation: use_debug,
            info: VkInfo {
                backend,
                device: device_name,
                validation: use_debug,
            },
            serial: 0,
            pixel: [0, 0, 0, 0],
        })
    }

    pub fn info(&self) -> VkInfo {
        self.info.clone()
    }

    pub fn validation_errors(&self) -> u32 {
        self.validation_errors.load(Ordering::Relaxed)
    }

    pub fn submit_offscreen(
        &mut self,
        clear: [f32; 4],
        draw_first: i32,
        draw_count: i32,
    ) -> Result<u32, String> {
        unsafe {
            self.device
                .reset_fences(&[self.fence])
                .map_err(|e| format!("vkResetFences: {e}"))?;
            self.device
                .reset_command_buffer(self.command_buffer, vk::CommandBufferResetFlags::empty())
                .map_err(|e| format!("vkResetCommandBuffer: {e}"))?;
            self.device
                .begin_command_buffer(
                    self.command_buffer,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(|e| format!("vkBeginCommandBuffer: {e}"))?;

            let clear_value = vk::ClearValue {
                color: vk::ClearColorValue { float32: clear },
            };
            let area = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: WIDTH,
                    height: HEIGHT,
                },
            };
            let begin = vk::RenderPassBeginInfo::default()
                .render_pass(self.render_pass)
                .framebuffer(self.framebuffer)
                .render_area(area)
                .clear_values(slice::from_ref(&clear_value));
            self.device.cmd_begin_render_pass(
                self.command_buffer,
                &begin,
                vk::SubpassContents::INLINE,
            );
            self.device.cmd_bind_pipeline(
                self.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline,
            );
            // Viewport/scissor are static in the pipeline; do not call the
            // dynamic-state setters or validation emits VUID-vkCmdDraw-02859.
            if draw_count > 0 {
                self.device.cmd_draw(
                    self.command_buffer,
                    draw_count as u32,
                    1,
                    draw_first.max(0) as u32,
                    0,
                );
            }
            self.device.cmd_end_render_pass(self.command_buffer);

            let region = vk::BufferImageCopy::default()
                .buffer_offset(0)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(0)
                        .base_array_layer(0)
                        .layer_count(1),
                )
                .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                .image_extent(vk::Extent3D {
                    width: WIDTH,
                    height: HEIGHT,
                    depth: 1,
                });
            self.device.cmd_copy_image_to_buffer(
                self.command_buffer,
                self.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                self.readback_buffer,
                slice::from_ref(&region),
            );
            self.device
                .end_command_buffer(self.command_buffer)
                .map_err(|e| format!("vkEndCommandBuffer: {e}"))?;

            let submit = vk::SubmitInfo::default()
                .command_buffers(slice::from_ref(&self.command_buffer));
            self.device
                .queue_submit(self.queue, slice::from_ref(&submit), self.fence)
                .map_err(|e| format!("vkQueueSubmit: {e}"))?;
            self.device
                .wait_for_fences(slice::from_ref(&self.fence), true, u64::MAX)
                .map_err(|e| format!("vkWaitForFences: {e}"))?;

            let size = (WIDTH * HEIGHT * 4) as vk::DeviceSize;
            let ptr = self
                .device
                .map_memory(
                    self.readback_memory,
                    0,
                    size,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(|e| format!("vkMapMemory: {e}"))?;
            let data = slice::from_raw_parts(ptr as *const u8, size as usize);
            let center = (((HEIGHT / 2) * WIDTH + WIDTH / 2) * 4) as usize;
            self.pixel.copy_from_slice(&data[center..center + 4]);
            self.device.unmap_memory(self.readback_memory);

            self.serial = self.serial.saturating_add(1);
            Ok(self.serial)
        }
    }

    pub fn read_center(&self) -> Result<[u8; 4], String> {
        Ok(self.pixel)
    }
}

impl Drop for Vk {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_buffer(self.readback_buffer, None);
            self.device.free_memory(self.readback_memory, None);
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_pipeline(self.pipeline, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device.destroy_shader_module(self.vert_module, None);
            self.device.destroy_shader_module(self.frag_module, None);
            self.device.destroy_framebuffer(self.framebuffer, None);
            self.device.destroy_render_pass(self.render_pass, None);
            self.device.destroy_image_view(self.image_view, None);
            self.device.destroy_image(self.image, None);
            self.device.free_memory(self.image_memory, None);
            if let (Some(du), Some(m)) = (self.debug_utils.as_ref(), self.debug_messenger) {
                du.destroy_debug_utils_messenger(m, None);
            }
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
            let _ = &self.physical;
        }
    }
}

fn read_spv(bytes: &[u8]) -> Result<Vec<u32>, String> {
    ash::util::read_spv(&mut Cursor::new(bytes)).map_err(|e| format!("read SPIR-V: {e}"))
}

fn cstr_array_to_string(arr: &[c_char]) -> String {
    unsafe {
        CStr::from_ptr(arr.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

fn find_memory_type(
    props: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    flags: vk::MemoryPropertyFlags,
) -> Option<u32> {
    (0..props.memory_type_count).find(|&i| {
        type_bits & (1 << i) != 0
            && props.memory_types[i as usize]
                .property_flags
                .contains(flags)
    })
}

unsafe fn has_layer(entry: &ash::Entry, name: &CStr) -> bool {
    entry
        .enumerate_instance_layer_properties()
        .map(|layers| {
            layers.iter().any(|l| {
                CStr::from_ptr(l.layer_name.as_ptr()) == name
            })
        })
        .unwrap_or(false)
}

unsafe fn has_instance_ext(entry: &ash::Entry, name: &CStr) -> bool {
    entry
        .enumerate_instance_extension_properties(None)
        .map(|exts| {
            exts.iter().any(|e| {
                CStr::from_ptr(e.extension_name.as_ptr()) == name
            })
        })
        .unwrap_or(false)
}

unsafe extern "system" fn debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _types: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    user_data: *mut c_void,
) -> vk::Bool32 {
    if !user_data.is_null() {
        let count = &*(user_data as *const AtomicU32);
        if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
            count.fetch_add(1, Ordering::Relaxed);
        }
    }
    if let Some(data) = data.as_ref() {
        if !data.p_message.is_null() {
            let msg = CStr::from_ptr(data.p_message).to_string_lossy();
            if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
                eprintln!("[vk validation ERROR] {msg}");
            }
        }
    }
    vk::FALSE
}
