//! Milestone 3: the frames in between, made on the graphics card.
//!
//! Each game frame is copied (blit, which also turns sRGB into linear) into one of two half-float images; the other holds the
//! previous game frame. Three compute passes follow (shaders/): `luma` (brightness at a quarter size, kept for the next
//! frame too), `motion` (a vector per 16x16 block: where it came from) and, for each added frame, `interp` (both frames
//! sampled along the vector at time t, blended). The result is blitted into the swapchain image the layer acquired.

use ash::vk;
use std::io::Cursor;

pub const FRAME_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;
const LUMA_FORMAT: vk::Format = vk::Format::R32_SFLOAT;
const FLOW_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;
pub const BLOCK: u32 = 16;

pub struct Img {
    pub image: vk::Image,
    memory: vk::DeviceMemory,
    pub view: vk::ImageView,
}

struct Pass {
    set_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
}

pub struct Gen {
    pub frames: [Img; 2],
    luma: [Img; 2],
    flow: Img,
    /// the flow after the vector median
    flow2: Img,
    pub out: Img,
    sampler: vk::Sampler,
    pool: vk::DescriptorPool,
    passes: [Pass; 4],
    /// [luma for frame k, motion with k current, interp with k current, smooth] for k = 0, 1
    sets: [[vk::DescriptorSet; 4]; 2],
    pub extent: vk::Extent2D,
    /// which of `frames` the next game frame goes into
    pub cur: usize,
    pub have_prev: bool,
}

/// The size of the motion field for a frame: one vector per BLOCK x BLOCK pixels.
pub fn flow_size(e: vk::Extent2D) -> vk::Extent2D {
    vk::Extent2D { width: e.width.div_ceil(BLOCK), height: e.height.div_ceil(BLOCK) }
}

pub fn quarter(e: vk::Extent2D) -> vk::Extent2D {
    vk::Extent2D { width: e.width.div_ceil(4), height: e.height.div_ceil(4) }
}

/// Work groups of 8x8 covering w x h.
pub fn groups(e: vk::Extent2D) -> (u32, u32) {
    (e.width.div_ceil(8), e.height.div_ceil(8))
}

fn memory_type(props: &vk::PhysicalDeviceMemoryProperties, bits: u32, want: vk::MemoryPropertyFlags) -> Option<u32> {
    (0..props.memory_type_count).find(|&i| bits & (1 << i) != 0 && props.memory_types[i as usize].property_flags.contains(want))
}

unsafe fn image(dev: &ash::Device, props: &vk::PhysicalDeviceMemoryProperties, e: vk::Extent2D, format: vk::Format, usage: vk::ImageUsageFlags) -> Result<Img, vk::Result> {
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D { width: e.width, height: e.height, depth: 1 })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    let image = dev.create_image(&info, None)?;
    let req = dev.get_image_memory_requirements(image);
    let Some(ty) = memory_type(props, req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL) else {
        dev.destroy_image(image, None);
        return Err(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY);
    };
    let memory = dev.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty), None)?;
    dev.bind_image_memory(image, memory, 0)?;
    let view = dev.create_image_view(
        &vk::ImageViewCreateInfo::default().image(image).view_type(vk::ImageViewType::TYPE_2D).format(format).subresource_range(crate::color_range(1)),
        None,
    )?;
    Ok(Img { image, memory, view })
}

unsafe fn destroy_image(dev: &ash::Device, i: &Img) {
    dev.destroy_image_view(i.view, None);
    dev.destroy_image(i.image, None);
    dev.free_memory(i.memory, None);
}

/// A compute pass: its shader, `sampled` combined image samplers then `storage` storage images, and an optional push constant.
unsafe fn pass(dev: &ash::Device, spv: &[u8], sampled: u32, storage: u32, push: u32) -> Result<Pass, vk::Result> {
    let mut bindings = vec![];
    for b in 0..sampled + storage {
        let ty = if b < sampled { vk::DescriptorType::COMBINED_IMAGE_SAMPLER } else { vk::DescriptorType::STORAGE_IMAGE };
        bindings.push(vk::DescriptorSetLayoutBinding::default().binding(b).descriptor_type(ty).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE));
    }
    let set_layout = dev.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings), None)?;
    let ranges = [vk::PushConstantRange { stage_flags: vk::ShaderStageFlags::COMPUTE, offset: 0, size: push }];
    let sl = [set_layout];
    let mut li = vk::PipelineLayoutCreateInfo::default().set_layouts(&sl);
    if push > 0 {
        li = li.push_constant_ranges(&ranges);
    }
    let layout = dev.create_pipeline_layout(&li, None)?;
    let code = ash::util::read_spv(&mut Cursor::new(spv)).map_err(|_| vk::Result::ERROR_INVALID_SHADER_NV)?;
    let module = dev.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)?;
    let stage = vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::COMPUTE).module(module).name(c"main");
    let made = dev.create_compute_pipelines(vk::PipelineCache::null(), &[vk::ComputePipelineCreateInfo::default().stage(stage).layout(layout)], None);
    dev.destroy_shader_module(module, None);
    let pipeline = made.map_err(|(_, e)| e)?[0];
    Ok(Pass { set_layout, layout, pipeline })
}

fn w(set: vk::DescriptorSet, b: u32, storage: bool, info: &[vk::DescriptorImageInfo; 1]) -> vk::WriteDescriptorSet<'_> {
    vk::WriteDescriptorSet::default()
        .dst_set(set)
        .dst_binding(b)
        .descriptor_type(if storage { vk::DescriptorType::STORAGE_IMAGE } else { vk::DescriptorType::COMBINED_IMAGE_SAMPLER })
        .image_info(info)
}

static LUMA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/luma.spv"));
static MOTION: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/motion.spv"));
static INTERP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/interp.spv"));
static SMOOTH: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/smooth.spv"));

impl Gen {
    pub unsafe fn new(dev: &ash::Device, props: &vk::PhysicalDeviceMemoryProperties, extent: vk::Extent2D) -> Result<Gen, vk::Result> {
        let frame_usage = vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED;
        let frames = [image(dev, props, extent, FRAME_FORMAT, frame_usage)?, image(dev, props, extent, FRAME_FORMAT, frame_usage)?];
        let q = quarter(extent);
        let su = vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED;
        let luma = [image(dev, props, q, LUMA_FORMAT, su)?, image(dev, props, q, LUMA_FORMAT, su)?];
        let flow = image(dev, props, flow_size(extent), FLOW_FORMAT, su)?;
        let flow2 = image(dev, props, flow_size(extent), FLOW_FORMAT, su)?;
        let out = image(dev, props, extent, FRAME_FORMAT, vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC)?;
        let sampler = dev.create_sampler(
            &vk::SamplerCreateInfo::default()
                .mag_filter(vk::Filter::LINEAR)
                .min_filter(vk::Filter::LINEAR)
                .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
            None,
        )?;
        let passes = [pass(dev, LUMA, 1, 1, 0)?, pass(dev, MOTION, 4, 1, 0)?, pass(dev, INTERP, 3, 1, 8)?, pass(dev, SMOOTH, 1, 1, 0)?];
        let sizes = [
            vk::DescriptorPoolSize { ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER, descriptor_count: 2 * (1 + 4 + 3 + 1) },
            vk::DescriptorPoolSize { ty: vk::DescriptorType::STORAGE_IMAGE, descriptor_count: 2 * 4 },
        ];
        let pool = dev.create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(8).pool_sizes(&sizes), None)?;
        let layouts = [passes[0].set_layout, passes[1].set_layout, passes[2].set_layout, passes[3].set_layout];
        let mut sets = [[vk::DescriptorSet::null(); 4]; 2];
        for (k, s) in sets.iter_mut().enumerate() {
            let made = dev.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&layouts))?;
            s.copy_from_slice(&made);
            let (cur, prev) = (k, 1 - k);
            let sampled = |v: vk::ImageView| [vk::DescriptorImageInfo { sampler, image_view: v, image_layout: vk::ImageLayout::GENERAL }];
            let storage = |v: vk::ImageView| [vk::DescriptorImageInfo { sampler: vk::Sampler::null(), image_view: v, image_layout: vk::ImageLayout::GENERAL }];
            // luma: frame k -> luma k
            let l0 = sampled(frames[cur].view);
            let l1 = storage(luma[cur].view);
            // motion: luma prev, luma cur, frame prev, frame cur -> flow
            let m0 = sampled(luma[prev].view);
            let m1 = sampled(luma[cur].view);
            let m2 = sampled(frames[prev].view);
            let m3 = sampled(frames[cur].view);
            let m4 = storage(flow.view);
            // interp: frame prev, frame cur, flow -> out
            let i0 = sampled(frames[prev].view);
            let i1 = sampled(frames[cur].view);
            let i2 = sampled(flow2.view);
            let s0 = sampled(flow.view);
            let s1 = storage(flow2.view);
            let i3 = storage(out.view);
            let writes = [
                w(s[0], 0, false, &l0),
                w(s[0], 1, true, &l1),
                w(s[1], 0, false, &m0),
                w(s[1], 1, false, &m1),
                w(s[1], 2, false, &m2),
                w(s[1], 3, false, &m3),
                w(s[1], 4, true, &m4),
                w(s[2], 0, false, &i0),
                w(s[2], 1, false, &i1),
                w(s[2], 2, false, &i2),
                w(s[2], 3, true, &i3),
                w(s[3], 0, false, &s0),
                w(s[3], 1, true, &s1),
            ];
            dev.update_descriptor_sets(&writes, &[]);
        }
        Ok(Gen { frames, luma, flow, flow2, out, sampler, pool, passes, sets, extent, cur: 0, have_prev: false })
    }

    pub unsafe fn destroy(&self, dev: &ash::Device) {
        for p in &self.passes {
            dev.destroy_pipeline(p.pipeline, None);
            dev.destroy_pipeline_layout(p.layout, None);
            dev.destroy_descriptor_set_layout(p.set_layout, None);
        }
        dev.destroy_descriptor_pool(self.pool, None);
        dev.destroy_sampler(self.sampler, None);
        for i in self.frames.iter().chain(self.luma.iter()).chain([&self.flow, &self.flow2, &self.out]) {
            destroy_image(dev, i);
        }
    }

    fn compute_barrier(image: vk::Image, from: vk::ImageLayout, src: vk::AccessFlags, dst: vk::AccessFlags) -> vk::ImageMemoryBarrier<'static> {
        crate::barrier(image, from, vk::ImageLayout::GENERAL, src, dst, 1)
    }

    /// Records: the current frame's brightness, and (when there is a previous frame) the motion between the two. The current
    /// frame must already be in `frames[cur]` in GENERAL.
    /// `queries` (or null): timestamps 2, 3 and 4 are written after the brightness, motion and smoothing passes.
    pub unsafe fn record_analysis(&self, dev: &ash::Device, cmd: vk::CommandBuffer, queries: vk::QueryPool) {
        let stamp = |i: u32| {
            if queries != vk::QueryPool::null() {
                dev.cmd_write_timestamp(cmd, vk::PipelineStageFlags::BOTTOM_OF_PIPE, queries, i);
            }
        };
        let k = self.cur;
        let q = quarter(self.extent);
        let pre = [Self::compute_barrier(self.luma[k].image, vk::ImageLayout::UNDEFINED, vk::AccessFlags::empty(), vk::AccessFlags::SHADER_WRITE)];
        dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &pre);
        self.dispatch(dev, cmd, 0, groups(q));
        stamp(2);
        let mut b = vec![Self::compute_barrier(self.luma[k].image, vk::ImageLayout::GENERAL, vk::AccessFlags::SHADER_WRITE, vk::AccessFlags::SHADER_READ)];
        if !self.have_prev {
            dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &b);
            stamp(3);
            stamp(4);
            return;
        }
        b.push(Self::compute_barrier(self.flow.image, vk::ImageLayout::UNDEFINED, vk::AccessFlags::empty(), vk::AccessFlags::SHADER_WRITE));
        dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &b);
        // one work group of 64 threads per block
        let f = flow_size(self.extent);
        self.dispatch(dev, cmd, 1, (f.width, f.height));
        stamp(3);
        let mid = [
            Self::compute_barrier(self.flow.image, vk::ImageLayout::GENERAL, vk::AccessFlags::SHADER_WRITE, vk::AccessFlags::SHADER_READ),
            Self::compute_barrier(self.flow2.image, vk::ImageLayout::UNDEFINED, vk::AccessFlags::empty(), vk::AccessFlags::SHADER_WRITE),
        ];
        dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &mid);
        self.dispatch(dev, cmd, 3, groups(flow_size(self.extent)));
        stamp(4);
        let after = [Self::compute_barrier(self.flow2.image, vk::ImageLayout::GENERAL, vk::AccessFlags::SHADER_WRITE, vk::AccessFlags::SHADER_READ)];
        dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &after);
    }

    /// Records one frame at time `t` into `out` (left in GENERAL, ready to be read by a transfer).
    pub unsafe fn record_frame(&self, dev: &ash::Device, cmd: vk::CommandBuffer, t: f32, first: bool) {
        let (from, src) = if first { (vk::ImageLayout::UNDEFINED, vk::AccessFlags::empty()) } else { (vk::ImageLayout::GENERAL, vk::AccessFlags::TRANSFER_READ) };
        let pre = [Self::compute_barrier(self.out.image, from, src, vk::AccessFlags::SHADER_WRITE)];
        dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::TRANSFER | vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &pre);
        let mut push = [0u8; 8];
        push[..4].copy_from_slice(&t.to_ne_bytes());
        push[4..].copy_from_slice(&crate::debug_mode().to_ne_bytes());
        dev.cmd_push_constants(cmd, self.passes[2].layout, vk::ShaderStageFlags::COMPUTE, 0, &push);
        self.dispatch(dev, cmd, 2, groups(self.extent));
        let after = [Self::compute_barrier(self.out.image, vk::ImageLayout::GENERAL, vk::AccessFlags::SHADER_WRITE, vk::AccessFlags::TRANSFER_READ)];
        dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &after);
    }

    unsafe fn dispatch(&self, dev: &ash::Device, cmd: vk::CommandBuffer, which: usize, (x, y): (u32, u32)) {
        let p = &self.passes[which];
        dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, p.pipeline);
        dev.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::COMPUTE, p.layout, 0, &[self.sets[self.cur][which]], &[]);
        dev.cmd_dispatch(cmd, x, y, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_cover_the_frame() {
        let e = vk::Extent2D { width: 1920, height: 1080 };
        assert_eq!(flow_size(e), vk::Extent2D { width: 120, height: 68 }, "1080 is not a multiple of 16: the last row of blocks is partial");
        assert_eq!(quarter(e), vk::Extent2D { width: 480, height: 270 });
        assert_eq!(groups(vk::Extent2D { width: 500, height: 500 }), (63, 63));
    }

    #[test]
    fn the_shaders_are_spir_v() {
        for s in [LUMA, MOTION, INTERP, SMOOTH] {
            assert_eq!(u32::from_le_bytes(s[..4].try_into().unwrap()), 0x0723_0203);
        }
    }
}
