//! Uploading decoded [`Frame`]s onto the GPU as a sampleable texture.

use crate::frame::{Frame, FrameError, FRAME_FORMAT};

/// A GPU texture backing one neural material.
///
/// Created once at the material's resolution; [`NeuralTexture::upload`] swaps in
/// the latest decoded frame each tick. Sample it from a shader via [`view`].
pub struct NeuralTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl NeuralTexture {
    /// Allocate an `Rgba8Unorm` texture of the given size.
    pub fn create(device: &wgpu::Device, width: u32, height: u32, label: &str) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FRAME_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        NeuralTexture {
            texture,
            view,
            width,
            height,
        }
    }

    /// Copy a decoded frame's pixels into the texture. Fails if the frame's
    /// dimensions do not match the texture's.
    pub fn upload(&self, queue: &wgpu::Queue, frame: &Frame) -> Result<(), FrameError> {
        frame.validate()?;
        if frame.width != self.width || frame.height != self.height {
            return Err(FrameError::SizeMismatch {
                expected: self.width as usize * self.height as usize * 4,
                actual: frame.width as usize * frame.height as usize * 4,
            });
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * self.width),
                rows_per_image: Some(self.height),
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    /// The texture view, for building a sampler/bind group in a render pipeline.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}
