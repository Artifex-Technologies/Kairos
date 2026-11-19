# CLAUDE.md — Kairos

## 1. What this is

Kairos is our own frame generation for Labyrinth OS ("labyrinth-framegen" until 2026-11-07): an implicit Vulkan layer in Rust
(ash 0.38) that draws frames in between a game's frames from estimated motion (compute shaders), plus a per-game launcher.
Code: `src/loader.rs` (the loader's layer interface, vk_layer.h written out), `src/lib.rs` (the hooks: instances, devices,
queues, swapchains, present, and the added frames), `src/gen.rs` (the generator), `shaders/` (luma, motion, smooth, interp),
`bin/kairos` (the launcher). Design, milestones and measurements: [DESIGN.md](DESIGN.md).

## 2. Build and test

Memory-capped and at low priority:

```bash
systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 env CARGO_BUILD_JOBS=4 nice -n 19 cargo test   # no GPU
sh tests/launcher.sh                       # the launcher's per-game choices
systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 env CARGO_BUILD_JOBS=4 nice -n 19 scripts/dev-layers.sh
# 64-bit and 32-bit layers and both manifests in target/layers; then, in Yggdrasil's nested test desktop:
VK_ADD_IMPLICIT_LAYER_PATH=$PWD/target/layers ENABLE_KAIROS=1 KAIROS_LOG=1 vkcube
```

With `VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation` (vulkan-validation-layers) zero messages are expected. There is no CI
workflow in this repository; the package (`kairos-git`) is built from Labyrinth OS's recipe.

## 3. Rules

- **Privacy.** Never write a host name, network address, SSID, MAC address, serial or the owner's name into a commit or
  file; check the diff before pushing.
- **Push as you go.** Push every commit as you make it; don't ask first. Stage by explicit path; `git pull --rebase`
  before pushing.
- **Clean room.** Never read or copy lsfg-vk or Lossless Scaling code or shaders (lsfg-vk is CC BY-NC-ND). The method
  comes from public literature on motion-compensated interpolation and from open material such as AMD's FidelityFX SDK
  (MIT), with its notice kept if used.
- **Test in the nested test desktop** (Yggdrasil `scripts/screenshot-desktop.sh`) with vkcube; never inside the owner's own
  session's games.
- **Light, and Vulkan everywhere.** It must cost a fraction of a millisecond per game frame and run on Intel, AMD and NVIDIA
  alike; CUDA only for NVIDIA-only extras.
- MPL-2.0.

## 4. Contracts

All in [docs/CONTRACTS.md](docs/CONTRACTS.md): the per-game settings file `~/.config/kairos/games.conf` (Yggdrasil Settings >
Games writes it), the launcher `kairos %command%`, the layer names and the `KAIROS_*` variables. Kairos uses Yggdrasil's
nested test desktop for testing only.

## 5. Open work

docs/plans/BACKLOG.md and docs/plans/LOG.md (private; ask the owner).

## 6. Know-how

- Shaders are compiled for Vulkan 1.0 (SPIR-V 1.0) by `build.rs`, so games that ask for 1.0 (vkcube does) accept them.
- The installed manifests (`data/*.json`) point at /usr/lib and /usr/lib32; `scripts/dev-layers.sh` writes copies pointing
  at the build. From a build folder in Steam: `VK_ADD_IMPLICIT_LAYER_PATH=/path/to/Kairos/target/layers ENABLE_KAIROS=1
  %command%` (it reaches Proton games through Steam's container as it is).
- A Proton game's stderr goes nowhere anyone sees: use `KAIROS_LOG_FILE=path`. `KAIROS_MODE=copy` and
  `KAIROS_DEBUG=flow|mark` help compare and see the added frames.
- Screenshots cannot judge the added frames (grim captures whichever image the compositor holds); that takes eyes.
- The old names (`labyrinth-framegen`, `LABYRINTH_FRAMEGEN_*`, `~/.config/labyrinth-framegen/games.conf`) are still read.
