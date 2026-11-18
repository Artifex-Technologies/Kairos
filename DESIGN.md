# Kairos: design and milestones

Named Kairos on 2026-11-07 ("labyrinth-framegen" was its working name). Frame generation for Labyrinth OS: more frames on the screen than a game draws, made
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

A Vulkan layer (`VK_LAYER_KAIROS_x86_64` and `_x86`, implicit, loaded only with `ENABLE_KAIROS=1`). It covers
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

Settings: Settings > Games switches it per game (since milestone 6, through the launcher below). For testing, the environment:
`KAIROS_MULTIPLIER` (2 to 4), `KAIROS_LOG=1`
(rates every two seconds on stderr), `KAIROS_MODE=copy` (copies instead, to compare),
`KAIROS_DEBUG=flow` (the added frames show the motion as colour: grey still) or `=mark` (a magenta square in
their top left corner), `KAIROS_LOG_FILE=path` (the log appended to a file: a Proton game's stderr goes nowhere
anyone sees).

**Per game (the launcher):** `kairos %command%` in a game's Steam launch options, once. It reads
`~/.config/kairos/games.conf` (`<Steam app id> <off|2|3|4>` per line, `default <...>` for the rest; Settings >
Games writes it; the old `~/.config/labyrinth-framegen/games.conf` is read while the new one does not exist) and switches the layer on with that multiplier, or starts the game untouched. Steam gives the game its id
(SteamAppId, SteamGameId or STEAM_COMPAT_APP_ID). `tests/launcher.sh` checks it.

Packaged as `kairos-git` (Labyrinth_OS packages/pkgbuilds; it replaces `labyrinth-framegen-git`): /usr/lib and /usr/lib32
libraries, both manifests in /usr/share/vulkan/implicit_layer.d, /usr/bin/kairos, and /usr/bin/labyrinth-framegen pointing at it so
launch options written before the rename still start their game. The settings variables are `KAIROS_*`; the old
`LABYRINTH_FRAMEGEN_*` names are still read.

From a build folder instead (development):
`VK_ADD_IMPLICIT_LAYER_PATH=/path/to/Kairos/target/layers ENABLE_KAIROS=1 %command%`
It reaches Proton games through Steam's container (pressure-vessel) as it is.

## Milestones

| | What | State |
|---|---|---|
| 1 | The layer loads, sees instances, devices, swapchains and presents | done 2026-11-05 |
| 2 | Extra presents per game frame, as copies: plumbing and pacing | done 2026-11-05: vkcube 90 game fps / 180 shown at x2, 60 / 180 at x3 (180 Hz) |
| 3 | Real frames in between: motion estimation and interpolation (compute shaders) | first version 2026-11-05: added frames are real in-between frames (vkcube: a clean cube, slightly soft edges), no validation errors; holds 90/180 at 1080p, but the card is about 87 % busy: far too heavy |
| 4 | Measured and made light | in progress. GPU timestamps per step (`KAIROS_LOG=1`). At 1080p the motion search took 8.0 ms of 8.3: one thread per block left the card idle. Now one work group of 64 threads per block (search area in shared memory, a reduction): **0.31 ms**; the whole generator **0.64 ms per game frame** (copy in 0.07, brightness 0.06, motion 0.31, smoothing 0.01, frames out 0.19), same motion quality. **First real game (2026-11-05): Stray** (Unreal Engine 4, DX11 through Proton Experimental, inside Steam's container): the layer loads into the game process, generates at 1080p for **about 0.6 ms** a game frame, 60 game fps (its menu cap) to 120 shown. Left: whether the added frames look right on a real screen (screenshots cannot tell: grim captures whichever image the compositor holds, 0 of 12 or 4 of 4 depending on timing, so this needs eyes), 1440p and 4K, latency (+1 game frame is inherent) |
| 5 | Second card | needs a second card to test |
| 6 | Settings > Games: per-game switch and multiplier; a package in Labyrinth's repository; a 32-bit build for old Proton games | **done 2026-11-05**: 32-bit build (Dead Space 2, 32-bit DirectX 9, at 3440x1440: about 1.1 ms a game frame); the launcher `kairos %command%` and `games.conf`; Settings > Games lists every installed Steam game with Off / 2x / 3x / 4x; package `labyrinth-framegen-git` (Labyrinth_OS recipe), published to Labyrinth's repository and offered in Agora as Frame generation (preview) |

**On a real screen (2026-11-06):** the owner played Dead Space 2 and Fallout 4 with the layer on; both ran smoothly.

## Several graphics chips: roles beyond frame generation (planned, 2026-11-18)

My direction: frame generation stays Kairos's main job, and it keeps working on its own for anyone who uses neither
Sarina nor Heimdall. On top of it, an **optional** part lets a computer with more than one graphics chip (a laptop with an
integrated and a dedicated one; later my desktop with two cards) give each chip a role:

- **Render** (the game's own card) and **frame generation** (the same card, or the second one: milestone 5).
- **Compute for Sarina**: her local model and speech run on the chip the game is not using (the integrated one on a
  laptop, the second card on the desktop), so a game keeps its card and Sarina stays responsive.
- **Compute for Heimdall**, where a job gains from a graphics chip.
- **Watching in real time** (meaning still to be settled with me; see the open questions).

Shape (to design properly before building):
- A small broker beside the layer (`kairos` already exists as the launcher): it lists the chips (Vulkan physical devices
  and DRM render nodes: vendor, integrated or dedicated, memory), knows which one a running game uses (the layer reports
  it), and answers "which chip may I use now?" to Sarina's and Heimdall's services, following game time
  (`yggdrasil-gametime`): during a game the game's card is reserved.
- It hands out a choice, never a capability: each program still opens the chip itself (llama.cpp/Ollama device selection,
  ONNX Runtime's device id, Vulkan compute). The broker has no rights over other programs; a program that ignores it is no
  worse off than today.
- Nothing in the layer depends on the broker: with no broker, or no second chip, Kairos is exactly today's Kairos.
- Vulkan compute or wgpu first (runs on Intel, AMD and NVIDIA, and a Raspberry Pi has no use for it); CUDA only for
  NVIDIA-only extras (Sarina's references, `rust-cuda`).

Testing: the laptop (X1 Yoga Gen 8) has only an integrated Intel chip, so the "two chips" case needs other hardware. The
desktop's i9-9900K has an integrated Intel UHD 630 that is switched off in the BIOS today; switching it on (with the
monitor still on the NVIDIA card) would give a two-chip test machine now. Or wait for the second card.

My answers (2026-11-18):
1. **Both**, each its own opt-in feature: Sarina may look at a game's frames when I ask her (Kairos hands her a scaled-down
   copy of one frame; a visible indicator while she looks; never on her own, never in the background), and Heimdall's live
   network analysis may use the spare chip on LykanOS. Sarina's current model has no vision, so the first needs a vision
   model (a download: my call when we get there).
2. **The desktop's integrated Intel chip goes on** (BIOS: Settings > Advanced > Integrated Graphics Configuration:
   Initiate Graphic Adapter = PEG, IGD Multi-Monitor = Enabled; monitor stays on the NVIDIA card). NVIDIA plus Intel is
   also exactly my friend's mix (an RTX 2070 and an Intel Arc B580), so it tests the cross-vendor path: rendering on one
   vendor's card and generating or computing on the other's (Vulkan external memory through dma-buf, or host memory).
3. **Labyrinth OS first** (my desktop is the test machine), then LykanOS, once I have a laptop with both kinds of chip.

Vulkan everywhere: every role above is Vulkan (or wgpu) compute, so it runs on Intel, AMD and NVIDIA alike.

## Testing

- `cargo test` (no GPU).
- `scripts/dev-layers.sh` builds the 64-bit and 32-bit layers and writes both manifests to `target/layers`; then, in the
  nested test desktop (Yggdrasil `scripts/screenshot-desktop.sh`):
  `VK_ADD_IMPLICIT_LAYER_PATH=$PWD/target/layers ENABLE_KAIROS=1 KAIROS_LOG=1 vkcube`.
  The installed manifests (`data/*.json`) point at /usr/lib and /usr/lib32.
- With `VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation` (vulkan-validation-layers): zero messages expected.
- Shaders are compiled for Vulkan 1.0 (SPIR-V 1.0), so games that ask for 1.0 (vkcube does) accept them.
