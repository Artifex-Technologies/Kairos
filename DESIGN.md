# labyrinth-framegen: design and milestones

Working name until the owner names it. Frame generation for Labyrinth OS: more frames on the screen than a game draws, made
by estimating motion between two game frames and drawing the frames in between. Our own: no Lossless Scaling, no lsfg-vk.

## Why our own

- lsfg-vk (the Linux port of Lossless Scaling's frame generation) is CC BY-NC-ND 4.0: it cannot be changed or bundled, and
  it needs the paid Lossless Scaling app's files. It has no dual-graphics-card mode.
- This project is MPL-2.0 like the rest of the ecosystem, needs nothing to buy, and is built into Labyrinth: settings in
  Yggdrasil Settings > Games, a second card supported by design.

**Clean room:** lsfg-vk's and Lossless Scaling's code and shaders are never read or copied. The method comes from public
literature on motion-compensated frame interpolation (block matching, optical flow, occlusion handling) and from open
material such as AMD's FidelityFX SDK (MIT) if it is used, with its notice kept.

## How it works

A Vulkan layer (`VK_LAYER_LABYRINTH_framegen`, implicit, loaded only with `ENABLE_LABYRINTH_FRAMEGEN=1`). It covers
native Vulkan games and every Proton game (DXVK and VKD3D-Proton draw through Vulkan).

1. **Swapchain:** when the game makes its swapchain the layer asks for `multiplier` more images, copy usage, and FIFO
   presentation (one image per refresh, which is what spaces the frames evenly).
2. **Present:** for each game frame, the layer acquires `multiplier - 1` more images, draws the frames in between into
   them, presents them, then presents the game's frame. The game is held to refresh / multiplier; the screen gets the full
   refresh. One game frame of latency is inherent: the frames in between need the next game frame.
3. **Frames in between** (milestone 3): the game frame is blitted into the layer's own half-float images (sRGB becomes
   linear); four compute passes (shaders/): `luma` (brightness at a quarter size), `motion` (a vector per 16x16 block: an
   exhaustive +-48 px search at quarter size, then +-3 px at full size, with a small preference for less movement; one
   work group of 64 threads per block, the search area in shared memory),
   `smooth` (3x3 vector median, removing lone wrong vectors), and per added frame `interp` (both frames sampled along the
   vector at time t, blended; where they disagree, the nearer frame alone). The result is blitted into the acquired
   swapchain image.
4. **Second card** (milestone 5): the game draws on card A; the layer copies each frame to card B (external memory, or host
   memory as the fallback) and generates and presents there, with the monitor on card B.

Settings (environment for now, Settings > Games later): `LABYRINTH_FRAMEGEN_MULTIPLIER` (2 to 4), `LABYRINTH_FRAMEGEN_LOG=1`
(rates every two seconds on stderr), `LABYRINTH_FRAMEGEN_MODE=copy` (copies instead, to compare),
`LABYRINTH_FRAMEGEN_DEBUG=flow` (the added frames show the motion as colour: grey still) or `=mark` (a magenta square in
their top left corner, to pick them out in screenshots).

## Milestones

| | What | State |
|---|---|---|
| 1 | The layer loads, sees instances, devices, swapchains and presents | done 2026-11-05 |
| 2 | Extra presents per game frame, as copies: plumbing and pacing | done 2026-11-05: vkcube 90 game fps / 180 shown at x2, 60 / 180 at x3 (180 Hz) |
| 3 | Real frames in between: motion estimation and interpolation (compute shaders) | first version 2026-11-05: added frames are real in-between frames (vkcube: a clean cube, slightly soft edges), no validation errors; holds 90/180 at 1080p, but the card is about 87 % busy: far too heavy |
| 4 | Measured and made light | in progress. GPU timestamps per step (`LABYRINTH_FRAMEGEN_LOG=1`). At 1080p the motion search took 8.0 ms of 8.3: one thread per block left the card idle. Now one work group of 64 threads per block (search area in shared memory, a reduction): **0.31 ms**; the whole generator **0.64 ms per game frame** (copy in 0.07, brightness 0.06, motion 0.31, smoothing 0.01, frames out 0.19), same motion quality. Left: a real game; 1440p and 4K; latency (+1 game frame is inherent) |
| 5 | Second card | needs a second card to test |
| 6 | Settings > Games: per-game switch and multiplier; a package in Labyrinth's repository; a 32-bit build for old Proton games | |

## Testing

- `cargo test` (no GPU).
- In the nested test desktop (Yggdrasil `scripts/screenshot-desktop.sh`):
  `VK_ADD_IMPLICIT_LAYER_PATH=target/release ENABLE_LABYRINTH_FRAMEGEN=1 LABYRINTH_FRAMEGEN_LOG=1 vkcube`
  (`target/release/labyrinth_framegen.json` is `data/labyrinth_framegen.json` with `./` before the library name).
- With `VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation` (vulkan-validation-layers): zero messages expected.
- Shaders are compiled for Vulkan 1.0 (SPIR-V 1.0), so games that ask for 1.0 (vkcube does) accept them.
