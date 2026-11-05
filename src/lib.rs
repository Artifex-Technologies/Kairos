//! labyrinth-framegen: frame generation as a Vulkan layer (working name; see DESIGN.md).
//!
//! The layer sits between a game and the driver. It sees every frame the game presents (vkQueuePresentKHR) and, for each one,
//! presents `multiplier - 1` frames of its own first, so the screen shows more frames than the game draws.
//!
//! The frames in between are made from the previous game frame and this one along the estimated motion (gen.rs, shaders/).
//! FIFO presentation shows one image per refresh, so the game is held to refresh / multiplier and the screen gets the full
//! refresh. If the generator cannot be made (memory, formats), the added frames are copies of the game's frame.
//!
//! Off unless asked for: the manifest loads it only with ENABLE_LABYRINTH_FRAMEGEN=1. Settings (environment, for now):
//! LABYRINTH_FRAMEGEN_MULTIPLIER (2..4, default 2), LABYRINTH_FRAMEGEN_LOG=1 (rates every two seconds, on stderr),
//! LABYRINTH_FRAMEGEN_MODE=copy (copies instead of generated frames, to compare).

#![allow(clippy::missing_safety_doc)]

mod gen;
mod loader;

use ash::vk;
use ash::vk::Handle;
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use loader::*;

/// Frames in flight the layer keeps its own objects for.
const SLOTS: usize = 4;
const MAX_MULTIPLIER: u32 = 4;

pub fn multiplier() -> u32 {
    static M: OnceLock<u32> = OnceLock::new();
    *M.get_or_init(|| parse_multiplier(std::env::var("LABYRINTH_FRAMEGEN_MULTIPLIER").ok().as_deref()))
}

/// The multiplier asked for: 2 to MAX_MULTIPLIER, 2 when missing or not understood.
pub fn parse_multiplier(v: Option<&str>) -> u32 {
    v.and_then(|s| s.trim().parse::<u32>().ok()).filter(|m| (2..=MAX_MULTIPLIER).contains(m)).unwrap_or(2)
}

fn copy_mode() -> bool {
    static C: OnceLock<bool> = OnceLock::new();
    *C.get_or_init(|| std::env::var("LABYRINTH_FRAMEGEN_MODE").is_ok_and(|v| v == "copy"))
}

/// LABYRINTH_FRAMEGEN_DEBUG: `flow` shows the estimated motion as colour in the added frames, `mark` puts a magenta square in
/// their top left corner (for telling them apart in screenshots). 0 = off, 1 = flow, 2 = mark.
fn debug_mode() -> i32 {
    static D: OnceLock<i32> = OnceLock::new();
    *D.get_or_init(|| match std::env::var("LABYRINTH_FRAMEGEN_DEBUG").as_deref() {
        Ok("flow") => 1,
        Ok("mark") => 2,
        _ => 0,
    })
}

fn logging() -> bool {
    static L: OnceLock<bool> = OnceLock::new();
    *L.get_or_init(|| std::env::var("LABYRINTH_FRAMEGEN_LOG").is_ok_and(|v| v == "1"))
}

macro_rules! log {
    ($($t:tt)*) => { if logging() { log_line(&format!($($t)*)); } };
}

/// A log line on stderr, or appended to LABYRINTH_FRAMEGEN_LOG_FILE (a game under Proton has no stderr anyone sees).
fn log_line(msg: &str) {
    static FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    let file = FILE.get_or_init(|| {
        let path = std::env::var_os("LABYRINTH_FRAMEGEN_LOG_FILE")?;
        std::fs::OpenOptions::new().create(true).append(true).open(path).ok().map(Mutex::new)
    });
    let line = format!("[labyrinth-framegen {}] {msg}\n", std::process::id());
    match file {
        Some(f) => {
            use std::io::Write;
            let _ = f.lock().map(|mut f| f.write_all(line.as_bytes()));
        }
        None => eprint!("{line}"),
    }
}

/// What a dispatchable handle's loader table is: the key the loader itself uses to tell devices (and their queues) apart.
unsafe fn key<H: Handle>(h: H) -> usize {
    *(h.as_raw() as *const usize)
}

struct InstanceData {
    next_gipa: vk::PFN_vkGetInstanceProcAddr,
    inst: ash::Instance,
}

struct DeviceData {
    handle: vk::Device,
    next_gdpa: vk::PFN_vkGetDeviceProcAddr,
    dev: ash::Device,
    swap: ash::khr::swapchain::DeviceFn,
    set_loader_data: Option<SetDeviceLoaderData>,
    mem: vk::PhysicalDeviceMemoryProperties,
    /// nanoseconds per timestamp tick
    ts_period: f32,
    /// queue -> its family (from vkGetDeviceQueue / vkGetDeviceQueue2)
    families: HashMap<u64, u32>,
}

/// The layer's own objects for one frame in flight.
struct Slot {
    fence: vk::Fence,
    cmd: vk::CommandBuffer,
    /// one per generated frame: signalled when the image is acquired
    acquired: Vec<vk::Semaphore>,
    /// one per generated frame, and the last for the game's own frame: signalled when it may be shown
    ready: Vec<vk::Semaphore>,
    used: bool,
    /// GPU timestamps (only with LABYRINTH_FRAMEGEN_LOG=1): see TIMES
    queries: vk::QueryPool,
    timed: bool,
}

/// The timestamps written per game frame, and what lies between them.
const STAMPS: u32 = 6;
pub const TIMES: [&str; 5] = ["copy in", "brightness", "motion", "smoothing", "frames out"];

struct SwapchainData {
    device: usize,
    images: Vec<vk::Image>,
    extent: vk::Extent2D,
    layers: u32,
    /// made on the first present (the queue family is known then)
    pool: vk::CommandPool,
    slots: Vec<Slot>,
    gen: Option<gen::Gen>,
    gen_failed: bool,
    next: usize,
    /// rate counting
    since: Instant,
    game_frames: u32,
    shown_frames: u32,
    /// GPU time per step (ms, summed) and how many frames were timed
    gpu: [f64; 5],
    gpu_frames: u32,
}

#[derive(Default)]
struct State {
    instances: HashMap<usize, InstanceData>,
    devices: HashMap<usize, DeviceData>,
    swapchains: HashMap<u64, SwapchainData>,
}

fn state() -> MutexGuard<'static, State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::default())).lock().unwrap_or_else(|e| e.into_inner())
}

// --- the loader's entry point ---------------------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "system" fn vkNegotiateLoaderLayerInterfaceVersion(p: *mut NegotiateLayerInterface) -> vk::Result {
    let Some(n) = p.as_mut() else { return vk::Result::ERROR_INITIALIZATION_FAILED };
    if n.version < 2 {
        return vk::Result::ERROR_INITIALIZATION_FAILED;
    }
    n.version = 2;
    n.gipa = Some(get_instance_proc_addr);
    n.gdpa = Some(get_device_proc_addr);
    n.gpdpa = None;
    vk::Result::SUCCESS
}

fn hook(name: &CStr) -> vk::PFN_vkVoidFunction {
    unsafe {
        Some(match name.to_bytes() {
            b"vkGetInstanceProcAddr" => std::mem::transmute::<vk::PFN_vkGetInstanceProcAddr, unsafe extern "system" fn()>(get_instance_proc_addr),
            b"vkGetDeviceProcAddr" => std::mem::transmute::<vk::PFN_vkGetDeviceProcAddr, unsafe extern "system" fn()>(get_device_proc_addr),
            b"vkCreateInstance" => std::mem::transmute::<vk::PFN_vkCreateInstance, unsafe extern "system" fn()>(create_instance),
            b"vkDestroyInstance" => std::mem::transmute::<vk::PFN_vkDestroyInstance, unsafe extern "system" fn()>(destroy_instance),
            b"vkCreateDevice" => std::mem::transmute::<vk::PFN_vkCreateDevice, unsafe extern "system" fn()>(create_device),
            b"vkDestroyDevice" => std::mem::transmute::<vk::PFN_vkDestroyDevice, unsafe extern "system" fn()>(destroy_device),
            b"vkGetDeviceQueue" => std::mem::transmute::<vk::PFN_vkGetDeviceQueue, unsafe extern "system" fn()>(get_device_queue),
            b"vkGetDeviceQueue2" => std::mem::transmute::<vk::PFN_vkGetDeviceQueue2, unsafe extern "system" fn()>(get_device_queue2),
            b"vkCreateSwapchainKHR" => std::mem::transmute::<vk::PFN_vkCreateSwapchainKHR, unsafe extern "system" fn()>(create_swapchain),
            b"vkDestroySwapchainKHR" => std::mem::transmute::<vk::PFN_vkDestroySwapchainKHR, unsafe extern "system" fn()>(destroy_swapchain),
            b"vkQueuePresentKHR" => std::mem::transmute::<vk::PFN_vkQueuePresentKHR, unsafe extern "system" fn()>(queue_present),
            _ => return None,
        })
    }
}

const DEVICE_HOOKS: [&[u8]; 8] = [b"vkGetDeviceProcAddr", b"vkDestroyDevice", b"vkGetDeviceQueue", b"vkGetDeviceQueue2", b"vkCreateSwapchainKHR", b"vkDestroySwapchainKHR", b"vkQueuePresentKHR", b"vkCreateDevice"];

unsafe extern "system" fn get_instance_proc_addr(instance: vk::Instance, name: *const c_char) -> vk::PFN_vkVoidFunction {
    let n = CStr::from_ptr(name);
    if let Some(f) = hook(n) {
        return Some(f);
    }
    let next = state().instances.get(&key(instance)).map(|i| i.next_gipa);
    match next {
        Some(gipa) if !instance.is_null() => gipa(instance, name),
        _ => None,
    }
}

unsafe extern "system" fn get_device_proc_addr(device: vk::Device, name: *const c_char) -> vk::PFN_vkVoidFunction {
    let n = CStr::from_ptr(name);
    if DEVICE_HOOKS.contains(&n.to_bytes()) {
        if let Some(f) = hook(n) {
            return Some(f);
        }
    }
    let next = state().devices.get(&key(device)).map(|d| d.next_gdpa);
    next.and_then(|gdpa| gdpa(device, name))
}

// --- instances --------------------------------------------------------------------------------------------------------

unsafe extern "system" fn create_instance(info: *const vk::InstanceCreateInfo<'_>, alloc: *const vk::AllocationCallbacks<'_>, out: *mut vk::Instance) -> vk::Result {
    let Some(link) = find_instance_link(info) else { return vk::Result::ERROR_INITIALIZATION_FAILED };
    let next_gipa = (*(*link).u.cast::<LayerInstanceLink>()).next_gipa;
    // the next layer's links start after ours
    (*link).u = (*(*link).u.cast::<LayerInstanceLink>()).next.cast();
    let Some(create) = next_gipa(vk::Instance::null(), c"vkCreateInstance".as_ptr()) else { return vk::Result::ERROR_INITIALIZATION_FAILED };
    let create: vk::PFN_vkCreateInstance = std::mem::transmute(create);
    let r = create(info, alloc, out);
    if r != vk::Result::SUCCESS {
        return r;
    }
    let instance = *out;
    let inst = ash::Instance::load_with(|name| next_gipa(instance, name.as_ptr()).map_or(std::ptr::null(), |f| f as *const c_void), instance);
    state().instances.insert(key(instance), InstanceData { next_gipa, inst });
    log!("on (multiplier {}) in {}", multiplier(), std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default());
    r
}

unsafe extern "system" fn destroy_instance(instance: vk::Instance, alloc: *const vk::AllocationCallbacks<'_>) {
    let data = state().instances.remove(&key(instance));
    if let Some(d) = data {
        (d.inst.fp_v1_0().destroy_instance)(instance, alloc);
    }
}

// --- devices ----------------------------------------------------------------------------------------------------------

unsafe extern "system" fn create_device(pd: vk::PhysicalDevice, info: *const vk::DeviceCreateInfo<'_>, alloc: *const vk::AllocationCallbacks<'_>, out: *mut vk::Device) -> vk::Result {
    let Some(link) = find_device_link(info) else { return vk::Result::ERROR_INITIALIZATION_FAILED };
    let l = (*link).u.cast::<LayerDeviceLink>();
    let (next_gipa, next_gdpa) = ((*l).next_gipa, (*l).next_gdpa);
    (*link).u = (*l).next.cast();
    let set_loader_data = find_loader_data_callback(info);
    let Some(create) = next_gipa(vk::Instance::null(), c"vkCreateDevice".as_ptr()) else { return vk::Result::ERROR_INITIALIZATION_FAILED };
    let create: vk::PFN_vkCreateDevice = std::mem::transmute(create);
    let r = create(pd, info, alloc, out);
    if r != vk::Result::SUCCESS {
        return r;
    }
    let device = *out;
    let load = |name: &CStr| next_gdpa(device, name.as_ptr()).map_or(std::ptr::null(), |f| f as *const c_void);
    let dev = ash::Device::load_with(load, device);
    let swap = ash::khr::swapchain::DeviceFn::load(load);
    let mut s = state();
    // a physical device's loader table is its instance's
    let inst = s.instances.get(&key(pd)).map(|i| &i.inst);
    let mem = inst.map(|i| i.get_physical_device_memory_properties(pd)).unwrap_or_default();
    let ts_period = inst.map(|i| i.get_physical_device_properties(pd).limits.timestamp_period).unwrap_or(1.0);
    s.devices.insert(key(device), DeviceData { handle: device, next_gdpa, dev, swap, set_loader_data, mem, ts_period, families: HashMap::new() });
    r
}

unsafe extern "system" fn destroy_device(device: vk::Device, alloc: *const vk::AllocationCallbacks<'_>) {
    let mut s = state();
    let k = key(device);
    let owned: Vec<u64> = s.swapchains.iter().filter(|(_, sc)| sc.device == k).map(|(h, _)| *h).collect();
    let gone: Vec<SwapchainData> = owned.iter().filter_map(|h| s.swapchains.remove(h)).collect();
    if let Some(d) = s.devices.get(&k) {
        let _ = d.dev.device_wait_idle();
        for sc in &gone {
            free_swapchain_objects(d, sc);
        }
    }
    if let Some(d) = s.devices.remove(&k) {
        drop(s);
        (d.dev.fp_v1_0().destroy_device)(device, alloc);
    }
}

unsafe extern "system" fn get_device_queue(device: vk::Device, family: u32, index: u32, out: *mut vk::Queue) {
    let mut s = state();
    let Some(d) = s.devices.get_mut(&key(device)) else { return };
    (d.dev.fp_v1_0().get_device_queue)(device, family, index, out);
    if !(*out).is_null() {
        d.families.insert((*out).as_raw(), family);
    }
}

unsafe extern "system" fn get_device_queue2(device: vk::Device, info: *const vk::DeviceQueueInfo2<'_>, out: *mut vk::Queue) {
    let mut s = state();
    let Some(d) = s.devices.get_mut(&key(device)) else { return };
    (d.dev.fp_v1_1().get_device_queue2)(device, info, out);
    if !(*out).is_null() {
        d.families.insert((*out).as_raw(), (*info).queue_family_index);
    }
}

// --- swapchains -------------------------------------------------------------------------------------------------------

/// What the layer changes in a swapchain: room for the frames it adds, copies into and out of its images, and FIFO
/// presentation (one image per refresh: that is what spaces the added frames evenly).
pub fn adjust_swapchain(info: &mut vk::SwapchainCreateInfoKHR<'_>, multiplier: u32) {
    info.min_image_count += multiplier;
    info.image_usage |= vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST;
    info.present_mode = vk::PresentModeKHR::FIFO;
}

unsafe extern "system" fn create_swapchain(device: vk::Device, info: *const vk::SwapchainCreateInfoKHR<'_>, alloc: *const vk::AllocationCallbacks<'_>, out: *mut vk::SwapchainKHR) -> vk::Result {
    let mut s = state();
    let k = key(device);
    let Some(d) = s.devices.get(&k) else { return vk::Result::ERROR_INITIALIZATION_FAILED };
    let mut mine = *info;
    adjust_swapchain(&mut mine, multiplier());
    let mut r = (d.swap.create_swapchain_khr)(device, &mine, alloc, out);
    if r != vk::Result::SUCCESS {
        // the surface may not allow it (too many images, no copies): the game gets its swapchain as asked, untouched
        log!("swapchain as adjusted refused ({r:?}); this one is left alone");
        return (d.swap.create_swapchain_khr)(device, info, alloc, out);
    }
    let mut n = 0u32;
    r = (d.swap.get_swapchain_images_khr)(device, *out, &mut n, std::ptr::null_mut());
    let mut images = vec![vk::Image::null(); n as usize];
    if r == vk::Result::SUCCESS {
        r = (d.swap.get_swapchain_images_khr)(device, *out, &mut n, images.as_mut_ptr());
    }
    if r != vk::Result::SUCCESS {
        return r;
    }
    log!("swapchain {}x{}, {} images", mine.image_extent.width, mine.image_extent.height, n);
    s.swapchains.insert((*out).as_raw(), SwapchainData {
        device: k,
        images,
        extent: mine.image_extent,
        layers: mine.image_array_layers.max(1),
        pool: vk::CommandPool::null(),
        slots: vec![],
        gen: None,
        gen_failed: false,
        next: 0,
        since: Instant::now(),
        game_frames: 0,
        shown_frames: 0,
        gpu: [0.0; 5],
        gpu_frames: 0,
    });
    vk::Result::SUCCESS
}

unsafe fn free_swapchain_objects(d: &DeviceData, sc: &SwapchainData) {
    for slot in &sc.slots {
        d.dev.destroy_fence(slot.fence, None);
        for s in slot.acquired.iter().chain(slot.ready.iter()) {
            d.dev.destroy_semaphore(*s, None);
        }
        if slot.queries != vk::QueryPool::null() {
            d.dev.destroy_query_pool(slot.queries, None);
        }
    }
    if sc.pool != vk::CommandPool::null() {
        d.dev.destroy_command_pool(sc.pool, None);
    }
    if let Some(g) = &sc.gen {
        g.destroy(&d.dev);
    }
}

unsafe extern "system" fn destroy_swapchain(device: vk::Device, swapchain: vk::SwapchainKHR, alloc: *const vk::AllocationCallbacks<'_>) {
    let mut s = state();
    let sc = s.swapchains.remove(&swapchain.as_raw());
    let Some(d) = s.devices.get(&key(device)) else { return };
    if let Some(sc) = sc {
        let fences: Vec<vk::Fence> = sc.slots.iter().filter(|x| x.used).map(|x| x.fence).collect();
        if !fences.is_empty() {
            let _ = d.dev.wait_for_fences(&fences, true, u64::MAX);
        }
        free_swapchain_objects(d, &sc);
    }
    (d.swap.destroy_swapchain_khr)(device, swapchain, alloc);
}

/// The objects a swapchain's added frames need, made on its first present (on the presenting queue's family).
unsafe fn prepare(d: &DeviceData, sc: &mut SwapchainData, family: u32, m: u32) -> Result<(), vk::Result> {
    if sc.pool != vk::CommandPool::null() {
        return Ok(());
    }
    if !copy_mode() && sc.layers == 1 {
        match gen::Gen::new(&d.dev, &d.mem, sc.extent) {
            Ok(g) => {
                log!("generating frames at {}x{}", sc.extent.width, sc.extent.height);
                sc.gen = Some(g);
            }
            Err(e) => {
                log!("the frame generator could not be made ({e:?}); adding copies instead");
                sc.gen_failed = true;
            }
        }
    }
    sc.pool = d.dev.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(family).flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER), None)?;
    let cmds = d.dev.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(sc.pool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(SLOTS as u32))?;
    for cmd in cmds {
        // a command buffer is a dispatchable object: the loader's table goes into it, as for the game's own
        if let Some(set) = d.set_loader_data {
            let r = set(d.handle, cmd.as_raw() as *mut c_void);
            if r != vk::Result::SUCCESS {
                return Err(r);
            }
        }
        let sem = || d.dev.create_semaphore(&vk::SemaphoreCreateInfo::default(), None);
        let mut acquired = vec![];
        let mut ready = vec![];
        for _ in 0..m - 1 {
            acquired.push(sem()?);
            ready.push(sem()?);
        }
        ready.push(sem()?);
        let fence = d.dev.create_fence(&vk::FenceCreateInfo::default(), None)?;
        let queries = if logging() {
            d.dev.create_query_pool(&vk::QueryPoolCreateInfo::default().query_type(vk::QueryType::TIMESTAMP).query_count(STAMPS), None)?
        } else {
            vk::QueryPool::null()
        };
        sc.slots.push(Slot { fence, cmd, acquired, ready, used: false, queries, timed: false });
    }
    Ok(())
}

/// The whole image, same size on both sides.
fn blit_region(w: i32, h: i32) -> vk::ImageBlit {
    let sub = vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 };
    let corners = [vk::Offset3D { x: 0, y: 0, z: 0 }, vk::Offset3D { x: w, y: h, z: 1 }];
    vk::ImageBlit { src_subresource: sub, src_offsets: corners, dst_subresource: sub, dst_offsets: corners }
}

fn color_range(layers: u32) -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: layers }
}

fn barrier(image: vk::Image, from: vk::ImageLayout, to: vk::ImageLayout, src: vk::AccessFlags, dst: vk::AccessFlags, layers: u32) -> vk::ImageMemoryBarrier<'static> {
    vk::ImageMemoryBarrier::default()
        .image(image)
        .old_layout(from)
        .new_layout(to)
        .src_access_mask(src)
        .dst_access_mask(dst)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .subresource_range(color_range(layers))
}

unsafe extern "system" fn queue_present(queue: vk::Queue, info: *const vk::PresentInfoKHR<'_>) -> vk::Result {
    let mut s = state();
    let k = key(queue);
    let Some(present) = s.devices.get(&k).map(|d| d.swap.queue_present_khr) else { return vk::Result::ERROR_DEVICE_LOST };
    let p = &*info;
    // one swapchain at a time; anything else (or a swapchain the layer left alone) goes straight through
    if p.swapchain_count != 1 || !s.swapchains.contains_key(&(*p.p_swapchains).as_raw()) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        let count = p.swapchain_count;
        ONCE.call_once(|| log!("a present the layer leaves alone ({count} swapchains, or one it did not see made)"));
        drop(s);
        return present(queue, info);
    }
    let swapchain = *p.p_swapchains;
    let index = *p.p_image_indices;
    let waits: Vec<vk::Semaphore> = std::slice::from_raw_parts(p.p_wait_semaphores, p.wait_semaphore_count as usize).to_vec();
    let State { devices, swapchains, .. } = &mut *s;
    let d = devices.get(&k).unwrap();
    let sc = swapchains.get_mut(&swapchain.as_raw()).unwrap();
    let m = multiplier();
    let family = d.families.get(&queue.as_raw()).copied().unwrap_or(0);
    match add_frames(d, sc, queue, family, swapchain, index, &waits, m) {
        Ok(()) => {}
        Err(e) => {
            log!("could not add frames ({e:?}); this frame goes through as it is");
            drop(s);
            return present(queue, info);
        }
    }
    let slot = &sc.slots[(sc.next + SLOTS - 1) % SLOTS];
    let ready_real = *slot.ready.last().unwrap();
    // the game's own frame, last, after the added ones; its pNext (present ids, regions) is kept
    let mut mine = *p;
    mine.wait_semaphore_count = 1;
    mine.p_wait_semaphores = &ready_real;
    let r = present(queue, &mine);
    sc.game_frames += 1;
    sc.shown_frames += 1;
    let secs = sc.since.elapsed().as_secs_f32();
    if secs >= 2.0 {
        log!("game {:.0} fps, shown {:.0} fps", sc.game_frames as f32 / secs, sc.shown_frames as f32 / secs);
        if sc.gpu_frames > 0 {
            let n = sc.gpu_frames as f64;
            let parts: Vec<String> = TIMES.iter().zip(sc.gpu.iter()).map(|(name, t)| format!("{name} {:.3}", t / n)).collect();
            log!("GPU ms per game frame: {} (total {:.3})", parts.join(", "), sc.gpu.iter().sum::<f64>() / n);
            sc.gpu = [0.0; 5];
            sc.gpu_frames = 0;
        }
        sc.since = Instant::now();
        sc.game_frames = 0;
        sc.shown_frames = 0;
    }
    r
}

/// Acquires `m - 1` more images of the swapchain, copies the game's frame into each (milestone 2), and presents them; the
/// game's frame waits on the last `ready` semaphore of the slot used.
#[allow(clippy::too_many_arguments)]
unsafe fn add_frames(d: &DeviceData, sc: &mut SwapchainData, queue: vk::Queue, family: u32, swapchain: vk::SwapchainKHR, index: u32, waits: &[vk::Semaphore], m: u32) -> Result<(), vk::Result> {
    prepare(d, sc, family, m)?;
    let slot_i = sc.next;
    sc.next = (sc.next + 1) % SLOTS;
    let (used, fence, timed, queries) = { let s = &sc.slots[slot_i]; (s.used, s.fence, s.timed, s.queries) };
    if used {
        d.dev.wait_for_fences(&[fence], true, u64::MAX)?;
        d.dev.reset_fences(&[fence])?;
        if timed {
            let mut t = [0u64; STAMPS as usize];
            if d.dev.get_query_pool_results(queries, 0, &mut t, vk::QueryResultFlags::TYPE_64).is_ok() {
                for (i, sum) in sc.gpu.iter_mut().enumerate() {
                    *sum += t[i + 1].saturating_sub(t[i]) as f64 * d.ts_period as f64 / 1e6;
                }
                sc.gpu_frames += 1;
            }
        }
    }
    let slot = &mut sc.slots[slot_i];
    slot.timed = false;
    let mut targets = vec![];
    for k in 0..(m - 1) as usize {
        let mut j = 0u32;
        let r = (d.swap.acquire_next_image_khr)(d.handle, swapchain, u64::MAX, slot.acquired[k], vk::Fence::null(), &mut j);
        if r != vk::Result::SUCCESS && r != vk::Result::SUBOPTIMAL_KHR {
            return Err(r);
        }
        targets.push(j);
    }
    let game_image = sc.images[index as usize];
    let cmd = slot.cmd;
    d.dev.reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())?;
    d.dev.begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT))?;
    let layers = sc.layers;
    let (w, h) = (sc.extent.width as i32, sc.extent.height as i32);
    match sc.gen.as_mut() {
        Some(g) => {
            // the game's frame into the generator's own image (a blit: sRGB becomes linear half floats)
            let frame = g.frames[g.cur].image;
            if slot.queries != vk::QueryPool::null() {
                d.dev.cmd_reset_query_pool(cmd, slot.queries, 0, STAMPS);
                d.dev.cmd_write_timestamp(cmd, vk::PipelineStageFlags::TOP_OF_PIPE, slot.queries, 0);
                slot.timed = g.have_prev;
            }
            let before = [
                barrier(game_image, vk::ImageLayout::PRESENT_SRC_KHR, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::AccessFlags::MEMORY_WRITE, vk::AccessFlags::TRANSFER_READ, layers),
                barrier(frame, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::AccessFlags::empty(), vk::AccessFlags::TRANSFER_WRITE, 1),
            ];
            d.dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::ALL_COMMANDS, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &before);
            d.dev.cmd_blit_image(cmd, game_image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, frame, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[blit_region(w, h)], vk::Filter::NEAREST);
            let ready = [barrier(frame, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::GENERAL, vk::AccessFlags::TRANSFER_WRITE, vk::AccessFlags::SHADER_READ, 1)];
            d.dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &ready);
            if slot.queries != vk::QueryPool::null() {
                d.dev.cmd_write_timestamp(cmd, vk::PipelineStageFlags::BOTTOM_OF_PIPE, slot.queries, 1);
            }
            g.record_analysis(&d.dev, cmd, slot.queries);
            for (k, &j) in targets.iter().enumerate() {
                // the first game frame has nothing before it: its added frames are copies
                let (src, src_layout) = if g.have_prev {
                    g.record_frame(&d.dev, cmd, (k + 1) as f32 / m as f32, k == 0);
                    (g.out.image, vk::ImageLayout::GENERAL)
                } else {
                    (game_image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                };
                let target = sc.images[j as usize];
                let pre = [barrier(target, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::AccessFlags::empty(), vk::AccessFlags::TRANSFER_WRITE, layers)];
                d.dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &pre);
                d.dev.cmd_blit_image(cmd, src, src_layout, target, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[blit_region(w, h)], vk::Filter::NEAREST);
                let post = [barrier(target, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::PRESENT_SRC_KHR, vk::AccessFlags::TRANSFER_WRITE, vk::AccessFlags::empty(), layers)];
                d.dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::BOTTOM_OF_PIPE, vk::DependencyFlags::empty(), &[], &[], &post);
            }
            let back = [barrier(game_image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::ImageLayout::PRESENT_SRC_KHR, vk::AccessFlags::TRANSFER_READ, vk::AccessFlags::empty(), layers)];
            d.dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::BOTTOM_OF_PIPE, vk::DependencyFlags::empty(), &[], &[], &back);
            if slot.queries != vk::QueryPool::null() {
                d.dev.cmd_write_timestamp(cmd, vk::PipelineStageFlags::BOTTOM_OF_PIPE, slot.queries, 5);
            }
            // this frame is the previous one for the next
            g.cur = 1 - g.cur;
            g.have_prev = true;
        }
        None => {
            let layers = sc.layers;
            let mut before = vec![barrier(game_image, vk::ImageLayout::PRESENT_SRC_KHR, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::AccessFlags::MEMORY_WRITE, vk::AccessFlags::TRANSFER_READ, layers)];
            for &j in &targets {
                before.push(barrier(sc.images[j as usize], vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::AccessFlags::empty(), vk::AccessFlags::TRANSFER_WRITE, layers));
            }
            d.dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::ALL_COMMANDS, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &before);
            let sub = vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: layers };
            let region = vk::ImageCopy { src_subresource: sub, src_offset: vk::Offset3D::default(), dst_subresource: sub, dst_offset: vk::Offset3D::default(), extent: vk::Extent3D { width: sc.extent.width, height: sc.extent.height, depth: 1 } };
            for &j in &targets {
                d.dev.cmd_copy_image(cmd, game_image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, sc.images[j as usize], vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[region]);
            }
            let mut after = vec![barrier(game_image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::ImageLayout::PRESENT_SRC_KHR, vk::AccessFlags::TRANSFER_READ, vk::AccessFlags::empty(), layers)];
            for &j in &targets {
                after.push(barrier(sc.images[j as usize], vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::PRESENT_SRC_KHR, vk::AccessFlags::TRANSFER_WRITE, vk::AccessFlags::empty(), layers));
            }
            d.dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::BOTTOM_OF_PIPE, vk::DependencyFlags::empty(), &[], &[], &after);
        }
    }
    d.dev.end_command_buffer(cmd)?;
    let mut wait_sems: Vec<vk::Semaphore> = waits.to_vec();
    let mut wait_stages = vec![vk::PipelineStageFlags::ALL_COMMANDS; waits.len()];
    for k in 0..(m - 1) as usize {
        wait_sems.push(slot.acquired[k]);
        wait_stages.push(vk::PipelineStageFlags::TRANSFER);
    }
    let cmds = [cmd];
    let submit = vk::SubmitInfo::default().wait_semaphores(&wait_sems).wait_dst_stage_mask(&wait_stages).command_buffers(&cmds).signal_semaphores(&slot.ready);
    d.dev.queue_submit(queue, &[submit], slot.fence)?;
    slot.used = true;
    // the added frames, in order, before the game's own (FIFO shows one per refresh)
    for (k, &j) in targets.iter().enumerate() {
        let sw = [swapchain];
        let ix = [j];
        let w = [slot.ready[k]];
        let pi = vk::PresentInfoKHR::default().wait_semaphores(&w).swapchains(&sw).image_indices(&ix);
        let r = (d.swap.queue_present_khr)(queue, &pi);
        if r != vk::Result::SUCCESS && r != vk::Result::SUBOPTIMAL_KHR {
            return Err(r);
        }
        sc.shown_frames += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_multiplier_is_two_to_four() {
        assert_eq!(parse_multiplier(None), 2);
        assert_eq!(parse_multiplier(Some("3")), 3);
        assert_eq!(parse_multiplier(Some(" 4 ")), 4);
        assert_eq!(parse_multiplier(Some("1")), 2, "1 would add nothing");
        assert_eq!(parse_multiplier(Some("9")), 2, "more than four is not offered");
        assert_eq!(parse_multiplier(Some("x")), 2);
    }

    #[test]
    fn a_swapchain_gets_room_copies_and_fifo() {
        let mut i = vk::SwapchainCreateInfoKHR::default().min_image_count(3).image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT).present_mode(vk::PresentModeKHR::MAILBOX);
        adjust_swapchain(&mut i, 2);
        assert_eq!(i.min_image_count, 5);
        assert!(i.image_usage.contains(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST));
        assert_eq!(i.present_mode, vk::PresentModeKHR::FIFO);
    }
}
