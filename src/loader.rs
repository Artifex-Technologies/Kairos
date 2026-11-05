//! The Vulkan loader's layer interface (vk_layer.h), written out here: how a layer is handed the next layer's functions when
//! an instance or a device is made, and how it hands its own functions to the loader.

use ash::vk;
use std::ffi::c_void;

/// VkLayerFunction
pub const LAYER_LINK_INFO: i32 = 0;
pub const LOADER_DATA_CALLBACK: i32 = 1;

#[repr(C)]
pub struct LayerInstanceLink {
    pub next: *mut LayerInstanceLink,
    pub next_gipa: vk::PFN_vkGetInstanceProcAddr,
    pub next_gpdpa: *const c_void,
}

#[repr(C)]
pub struct LayerDeviceLink {
    pub next: *mut LayerDeviceLink,
    pub next_gipa: vk::PFN_vkGetInstanceProcAddr,
    pub next_gdpa: vk::PFN_vkGetDeviceProcAddr,
}

/// VkLayerInstanceCreateInfo and VkLayerDeviceCreateInfo: `u` is a union (the link, or a loader callback).
#[repr(C)]
pub struct LayerCreateInfo {
    pub s_type: vk::StructureType,
    pub p_next: *const c_void,
    pub function: i32,
    pub u: *mut c_void,
}

pub type SetDeviceLoaderData = unsafe extern "system" fn(vk::Device, *mut c_void) -> vk::Result;

/// VkNegotiateLayerInterface
#[repr(C)]
pub struct NegotiateLayerInterface {
    pub s_type: i32,
    pub p_next: *mut c_void,
    pub version: u32,
    pub gipa: Option<vk::PFN_vkGetInstanceProcAddr>,
    pub gdpa: Option<vk::PFN_vkGetDeviceProcAddr>,
    pub gpdpa: Option<unsafe extern "system" fn()>,
}

unsafe fn find(mut p: *const c_void, s_type: vk::StructureType, function: i32) -> Option<*mut LayerCreateInfo> {
    while !p.is_null() {
        let info = p as *mut LayerCreateInfo;
        if (*info).s_type == s_type && (*info).function == function {
            return Some(info);
        }
        p = (*info).p_next;
    }
    None
}

pub unsafe fn find_instance_link(info: *const vk::InstanceCreateInfo<'_>) -> Option<*mut LayerCreateInfo> {
    find((*info).p_next, vk::StructureType::LOADER_INSTANCE_CREATE_INFO, LAYER_LINK_INFO)
}

pub unsafe fn find_device_link(info: *const vk::DeviceCreateInfo<'_>) -> Option<*mut LayerCreateInfo> {
    find((*info).p_next, vk::StructureType::LOADER_DEVICE_CREATE_INFO, LAYER_LINK_INFO)
}

pub unsafe fn find_loader_data_callback(info: *const vk::DeviceCreateInfo<'_>) -> Option<SetDeviceLoaderData> {
    find((*info).p_next, vk::StructureType::LOADER_DEVICE_CREATE_INFO, LOADER_DATA_CALLBACK).map(|i| std::mem::transmute::<*mut c_void, SetDeviceLoaderData>((*i).u))
}
