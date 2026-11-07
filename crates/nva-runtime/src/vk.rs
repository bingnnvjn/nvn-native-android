//! Offscreen Vulkan 1.1 backend for the #44 smoke path.
//!
//! One 64x64 `R8G8B8A8_UNORM` image, a classic render pass, a fullscreen
//! triangle, a fence and a host-visible readback buffer. No WSI, no swapchain:
//! the PC smoke target is headless (docs/ARCHITECTURE.md §3.6).
//!
//! The shape follows the ABI spike's evidence (`spike/abi-slice/RESULTS.md`),
//! not its code: one frame is recorded into a single command buffer, submitted
//! once, waited on through an explicit fence, then the centre pixel is read
//! back from the mapped buffer.

use std::ffi::{c_char, c_void, CStr};
use std::io::Cursor;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use ash::ext::debug_utils;
use ash::vk;

use crate::debug_filter;

pub const WIDTH: u32 = 64;
pub const HEIGHT: u32 = 64;
const FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;

const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";
const DEBUG_UTILS_EXTENSION: &CStr = c"VK_EXT_debug_utils";

/// Why creating the graphics object failed.
pub enum CreateError {
    /// No loader, no device, or no validation layer: the smoke oracle is
    /// unavailable, and `nva-verify smoke` maps this to exit code 3 (#37 D8).
    OracleUnavailable(String),
    /// The environment is there but a Vulkan call failed.
    Failed(String),
}

/// Why waiting for the fence failed.
pub enum WaitError {
    Timeout,
    Failed(String),
}

/// Handle to the offscreen Vulkan state.
///
/// There is exactly one frame in flight; `submit_frame` refuses to record a
/// second one until `wait_frame` has consumed the first.
pub struct Vk {
    // Drop order matters: `_entry` is declared first but every Vulkan object is
    // destroyed by the manual `Drop` impl before the fields are dropped, so the
    // library is still loaded when the destroy calls run.
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
    validation_available: bool,

    props: vk::PhysicalDeviceProperties,
    device_name: String,
    pixel_rgba: u32,
    serial: u32,
    submitted: bool,
}

impl Vk {
    pub fn new() -> Result<Self, CreateError> {
        unsafe { Self::new_unsafe() }
    }

    unsafe fn new_unsafe() -> Result<Self, CreateError> {
        let entry = ash::Entry::load()
            .map_err(|e| CreateError::OracleUnavailable(format!("load libvulkan: {e}")))?;

        let layer_available = has_layer(&entry, VALIDATION_LAYER);
        let debug_ext_available =
            layer_available && has_instance_extension(&entry, DEBUG_UTILS_EXTENSION);

        let app_name = c"nva-runtime";
        let app_info = vk::ApplicationInfo::default()
            .application_name(app_name)
            .application_version(1)
            .engine_name(app_name)
            .engine_version(1)
            .api_version(vk::make_api_version(0, 1, 1, 0));

        let layer_ptrs: Vec<*const c_char> = if layer_available {
            vec![VALIDATION_LAYER.as_ptr()]
        } else {
            Vec::new()
        };
        let extension_ptrs: Vec<*const c_char> = if debug_ext_available {
            vec![DEBUG_UTILS_EXTENSION.as_ptr()]
        } else {
            Vec::new()
        };
        let create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_layer_names(&layer_ptrs)
            .enabled_extension_names(&extension_ptrs);

        let instance = entry
            .create_instance(&create_info, None)
            .map_err(|e| CreateError::OracleUnavailable(format!("vkCreateInstance: {e}")))?;

        let validation_errors = Arc::new(AtomicU32::new(0));
        let debug_utils = if debug_ext_available {
            Some(debug_utils::Instance::new(&entry, &instance))
        } else {
            None
        };
        let debug_messenger = match debug_utils.as_ref() {
            Some(du) => {
                let callback_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
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
                    .user_data(Arc::as_ptr(&validation_errors) as *mut c_void);
                du.create_debug_utils_messenger(&callback_info, None).ok()
            }
            None => None,
        };
        // Counting only makes sense when the layer really loaded and its
        // messages have somewhere to arrive (#31). Otherwise the verifier must
        // say "unavailable" and exit 3, never "clean".
        let validation_available = debug_ext_available && debug_messenger.is_some();

        let physical_devices = instance.enumerate_physical_devices().map_err(|e| {
            CreateError::OracleUnavailable(format!("vkEnumeratePhysicalDevices: {e}"))
        })?;
        if physical_devices.is_empty() {
            return Err(CreateError::OracleUnavailable(
                "no Vulkan physical device".into(),
            ));
        }

        let mut chosen = None;
        for physical in physical_devices {
            let families = instance.get_physical_device_queue_family_properties(physical);
            if let Some((family, _)) = families
                .iter()
                .enumerate()
                .find(|(_, q)| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            {
                chosen = Some((physical, family as u32));
                break;
            }
        }
        let (physical, queue_family) = chosen.ok_or_else(|| {
            CreateError::OracleUnavailable("no Vulkan device with a graphics queue".into())
        })?;

        let props = instance.get_physical_device_properties(physical);
        let device_name = device_name_string(&props.device_name);

        let priorities = [1.0f32];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priorities);
        let device = instance
            .create_device(
                physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(std::slice::from_ref(&queue_info)),
                None,
            )
            .map_err(|e| CreateError::OracleUnavailable(format!("vkCreateDevice: {e}")))?;
        let queue = device.get_device_queue(queue_family, 0);

        let memory_properties = instance.get_physical_device_memory_properties(physical);

        // ---- render target --------------------------------------------
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
                    .usage(
                        vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
                    )
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )
            .map_err(|e| CreateError::Failed(format!("vkCreateImage: {e}")))?;
        let requirements = device.get_image_memory_requirements(image);
        let memory_type = find_memory_type(
            &memory_properties,
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )
        .ok_or_else(|| {
            CreateError::OracleUnavailable("no DEVICE_LOCAL memory for the colour image".into())
        })?;
        let image_memory = device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(memory_type),
                None,
            )
            .map_err(|e| CreateError::Failed(format!("allocate image memory: {e}")))?;
        device
            .bind_image_memory(image, image_memory, 0)
            .map_err(|e| CreateError::Failed(format!("vkBindImageMemory: {e}")))?;

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
            .map_err(|e| CreateError::Failed(format!("vkCreateImageView: {e}")))?;

        // ---- render pass + framebuffer --------------------------------
        let attachment = vk::AttachmentDescription::default()
            .format(FORMAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            // The command buffer copies the image to the readback buffer right
            // after the render pass, so it must already be transfer-source.
            .final_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
        let color_reference = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(std::slice::from_ref(&color_reference));
        let render_pass = device
            .create_render_pass(
                &vk::RenderPassCreateInfo::default()
                    .attachments(std::slice::from_ref(&attachment))
                    .subpasses(std::slice::from_ref(&subpass)),
                None,
            )
            .map_err(|e| CreateError::Failed(format!("vkCreateRenderPass: {e}")))?;
        let framebuffer = device
            .create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(render_pass)
                    .attachments(std::slice::from_ref(&image_view))
                    .width(WIDTH)
                    .height(HEIGHT)
                    .layers(1),
                None,
            )
            .map_err(|e| CreateError::Failed(format!("vkCreateFramebuffer: {e}")))?;

        // ---- pipeline -------------------------------------------------
        // The build script compiles these into OUT_DIR; the tree never carries
        // a SPIR-V binary (ci/guard bans the suffix and the magic).
        let vert_spirv = read_spirv(include_bytes!(concat!(env!("OUT_DIR"), "/tri.vert.spv")))?;
        let frag_spirv = read_spirv(include_bytes!(concat!(env!("OUT_DIR"), "/tri.frag.spv")))?;
        let vert_module = device
            .create_shader_module(
                &vk::ShaderModuleCreateInfo::default().code(&vert_spirv),
                None,
            )
            .map_err(|e| CreateError::Failed(format!("create vertex shader: {e}")))?;
        let frag_module = device
            .create_shader_module(
                &vk::ShaderModuleCreateInfo::default().code(&frag_spirv),
                None,
            )
            .map_err(|e| CreateError::Failed(format!("create fragment shader: {e}")))?;

        let main = c"main";
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vert_module)
                .name(main),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(frag_module)
                .name(main),
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
        let scissor = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(vk::Extent2D {
                width: WIDTH,
                height: HEIGHT,
            });
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(std::slice::from_ref(&viewport))
            .scissors(std::slice::from_ref(&scissor));
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
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
            .attachments(std::slice::from_ref(&blend_attachment));
        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default();

        let pipeline_layout = device
            .create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default(), None)
            .map_err(|e| CreateError::Failed(format!("vkCreatePipelineLayout: {e}")))?;
        // Viewport and scissor are static: calling the dynamic-state setters
        // without declaring them dynamic is a validation error (VUID-vkCmdDraw).
        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blend)
            .layout(pipeline_layout)
            .render_pass(render_pass)
            .subpass(0);
        let pipelines = device
            .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
            .map_err(|(_, e)| CreateError::Failed(format!("vkCreateGraphicsPipelines: {e}")))?;
        let pipeline = pipelines[0];

        // ---- command pool / fence / readback --------------------------
        let command_pool = device
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
            .map_err(|e| CreateError::Failed(format!("vkCreateCommandPool: {e}")))?;
        let command_buffer = device
            .allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
            .map_err(|e| CreateError::Failed(format!("vkAllocateCommandBuffers: {e}")))?[0];
        let fence = device
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .map_err(|e| CreateError::Failed(format!("vkCreateFence: {e}")))?;

        let readback_size = (WIDTH * HEIGHT * 4) as vk::DeviceSize;
        let readback_buffer = device
            .create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(readback_size)
                    .usage(vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
            .map_err(|e| CreateError::Failed(format!("vkCreateBuffer(readback): {e}")))?;
        let requirements = device.get_buffer_memory_requirements(readback_buffer);
        let memory_type = find_memory_type(
            &memory_properties,
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )
        .ok_or_else(|| {
            CreateError::OracleUnavailable(
                "no HOST_VISIBLE|HOST_COHERENT memory for the readback buffer".into(),
            )
        })?;
        let readback_memory = device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(memory_type),
                None,
            )
            .map_err(|e| CreateError::Failed(format!("allocate readback memory: {e}")))?;
        device
            .bind_buffer_memory(readback_buffer, readback_memory, 0)
            .map_err(|e| CreateError::Failed(format!("vkBindBufferMemory: {e}")))?;

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
            validation_errors,
            validation_available,
            props,
            device_name,
            pixel_rgba: 0,
            serial: 0,
            submitted: false,
        })
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn validation_available(&self) -> bool {
        self.validation_available
    }

    pub fn validation_errors(&self) -> u32 {
        self.validation_errors.load(Ordering::Relaxed)
    }

    pub fn pixel_rgba(&self) -> u32 {
        self.pixel_rgba
    }

    pub fn limits(&self) -> &vk::PhysicalDeviceProperties {
        &self.props
    }

    pub fn wait_idle(&mut self) -> Result<(), String> {
        unsafe {
            self.device
                .device_wait_idle()
                .map_err(|e| format!("vkDeviceWaitIdle: {e}"))?;
        }
        Ok(())
    }

    /// Record and submit one frame. Does not wait; `wait_frame` does.
    pub fn submit_frame(&mut self, clear: [f32; 4], draw_vertices: u32) -> Result<u32, String> {
        if self.submitted {
            return Err("previous frame was submitted but not waited".into());
        }
        unsafe {
            self.device
                .reset_fences(std::slice::from_ref(&self.fence))
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
            let render_area = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: WIDTH,
                    height: HEIGHT,
                },
            };
            let begin_info = vk::RenderPassBeginInfo::default()
                .render_pass(self.render_pass)
                .framebuffer(self.framebuffer)
                .render_area(render_area)
                .clear_values(std::slice::from_ref(&clear_value));
            self.device.cmd_begin_render_pass(
                self.command_buffer,
                &begin_info,
                vk::SubpassContents::INLINE,
            );
            self.device.cmd_bind_pipeline(
                self.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline,
            );
            if draw_vertices > 0 {
                self.device
                    .cmd_draw(self.command_buffer, draw_vertices, 1, 0, 0);
            }
            self.device.cmd_end_render_pass(self.command_buffer);

            let copy_region = vk::BufferImageCopy::default()
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
                std::slice::from_ref(&copy_region),
            );
            self.device
                .end_command_buffer(self.command_buffer)
                .map_err(|e| format!("vkEndCommandBuffer: {e}"))?;

            let submit_info = vk::SubmitInfo::default()
                .command_buffers(std::slice::from_ref(&self.command_buffer));
            self.device
                .queue_submit(self.queue, std::slice::from_ref(&submit_info), self.fence)
                .map_err(|e| format!("vkQueueSubmit: {e}"))?;
        }
        self.serial = self.serial.saturating_add(1);
        self.submitted = true;
        Ok(self.serial)
    }

    /// Wait for the submitted frame's fence, then read the centre pixel back.
    pub fn wait_frame(&mut self, timeout_ns: u64) -> Result<(), WaitError> {
        if !self.submitted {
            return Err(WaitError::Failed("no frame was submitted".into()));
        }
        unsafe {
            match self
                .device
                .wait_for_fences(std::slice::from_ref(&self.fence), true, timeout_ns)
            {
                Ok(()) => {}
                Err(vk::Result::TIMEOUT) => return Err(WaitError::Timeout),
                Err(e) => return Err(WaitError::Failed(format!("vkWaitForFences: {e}"))),
            }

            let size = (WIDTH * HEIGHT * 4) as vk::DeviceSize;
            let pointer = self
                .device
                .map_memory(self.readback_memory, 0, size, vk::MemoryMapFlags::empty())
                .map_err(|e| WaitError::Failed(format!("vkMapMemory: {e}")))?;
            let bytes = std::slice::from_raw_parts(pointer as *const u8, size as usize);
            let centre = (((HEIGHT / 2) * WIDTH + WIDTH / 2) * 4) as usize;
            self.pixel_rgba = u32::from_le_bytes([
                bytes[centre],
                bytes[centre + 1],
                bytes[centre + 2],
                bytes[centre + 3],
            ]);
            self.device.unmap_memory(self.readback_memory);
        }
        self.submitted = false;
        Ok(())
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
            if let (Some(debug_utils), Some(messenger)) =
                (self.debug_utils.as_ref(), self.debug_messenger)
            {
                debug_utils.destroy_debug_utils_messenger(messenger, None);
            }
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
        let _ = &self.physical;
    }
}

fn read_spirv(bytes: &[u8]) -> Result<Vec<u32>, CreateError> {
    ash::util::read_spv(&mut Cursor::new(bytes))
        .map_err(|e| CreateError::Failed(format!("read SPIR-V: {e}")))
}

fn device_name_string(name: &[c_char]) -> String {
    unsafe { CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned() }
}

fn find_memory_type(
    properties: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    flags: vk::MemoryPropertyFlags,
) -> Option<u32> {
    (0..properties.memory_type_count).find(|&index| {
        type_bits & (1 << index) != 0
            && properties.memory_types[index as usize]
                .property_flags
                .contains(flags)
    })
}

unsafe fn has_layer(entry: &ash::Entry, name: &CStr) -> bool {
    entry
        .enumerate_instance_layer_properties()
        .map(|layers| {
            layers
                .iter()
                .any(|layer| CStr::from_ptr(layer.layer_name.as_ptr()) == name)
        })
        .unwrap_or(false)
}

unsafe fn has_instance_extension(entry: &ash::Entry, name: &CStr) -> bool {
    entry
        .enumerate_instance_extension_properties(None)
        .map(|extensions| {
            extensions
                .iter()
                .any(|extension| CStr::from_ptr(extension.extension_name.as_ptr()) == name)
        })
        .unwrap_or(false)
}

unsafe extern "system" fn debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    types: vk::DebugUtilsMessageTypeFlagsEXT,
    callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    user_data: *mut c_void,
) -> vk::Bool32 {
    let Some(data) = callback_data.as_ref() else {
        return vk::FALSE;
    };
    let message_id = if data.p_message_id_name.is_null() {
        ""
    } else {
        CStr::from_ptr(data.p_message_id_name)
            .to_str()
            .unwrap_or("")
    };
    let message = if data.p_message.is_null() {
        ""
    } else {
        CStr::from_ptr(data.p_message).to_str().unwrap_or("")
    };

    let counts = debug_filter::counts_as_validation_error(
        severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR),
        types.contains(vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION),
        message_id,
        message,
    );
    if counts {
        if !user_data.is_null() {
            let counter = &*(user_data as *const AtomicU32);
            counter.fetch_add(1, Ordering::Relaxed);
        }
        eprintln!("[nva-runtime validation ERROR] {message_id}: {message}");
    } else if !message.is_empty() {
        eprintln!("[nva-runtime driver/non-error] {message_id}: {message}");
    }
    vk::FALSE
}
