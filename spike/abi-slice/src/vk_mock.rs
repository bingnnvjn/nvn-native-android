//! `--no-default-features` stand-in so `cargo check` works without glslang/Vulkan.
//! It is never used in the real PC/arm64/Android builds (feature `vk` is on).

#[derive(Clone, Debug)]
pub struct VkInfo {
    pub backend: String,
    pub device: String,
    pub validation: bool,
}

pub struct Vk {
    serial: u32,
    pixel: [u8; 4],
}

impl Vk {
    pub fn new(_enable_validation: bool) -> Result<Self, String> {
        Ok(Self {
            serial: 0,
            pixel: [0, 255, 0, 255],
        })
    }

    pub fn info(&self) -> VkInfo {
        VkInfo {
            backend: "mock-vk".into(),
            device: "mock".into(),
            validation: false,
        }
    }

    pub fn validation_errors(&self) -> u32 {
        0
    }

    pub fn submit_offscreen(
        &mut self,
        _clear: [f32; 4],
        _draw_first: i32,
        _draw_count: i32,
    ) -> Result<u32, String> {
        self.serial += 1;
        Ok(self.serial)
    }

    pub fn read_center(&self) -> Result<[u8; 4], String> {
        Ok(self.pixel)
    }
}
