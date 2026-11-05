# labyrinth-framegen (working name)

Our own frame generation for Labyrinth OS, as a Vulkan layer in Rust (ash 0.38). Plan and milestones: DESIGN.md.

- `src/loader.rs`: the loader's layer interface (vk_layer.h) written out; `src/lib.rs`: the hooks (instances, devices,
  queues, swapchains, present) and the added frames.
- Clean room: never read or copy lsfg-vk or Lossless Scaling code or shaders (lsfg-vk is CC BY-NC-ND).
- Test with vkcube in Yggdrasil's nested test desktop (DESIGN.md, Testing); never inside the owner's own session's games.
- MPL-2.0. Public repository: never write the PC's hostname or network details.
