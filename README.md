# Kairos

Frame generation for Labyrinth OS, as a Vulkan layer: more frames on the screen than the game draws, for native Vulkan
games and Proton games alike. Our own: it needs no other app, and it is meant to run the generation on a second graphics
card too. Named Kairos on 2026-11-07, for the Greek god of the right moment: the frames it adds are the moments between the
game's own (it was "labyrinth-framegen" until then).

**State (2026-11-05): an early preview.** The layer adds real in-between frames, made from estimated motion, at the right
moments: vkcube 90 game frames a second and 180 on the screen; Stray (Unreal Engine 4 through Proton) 60 to 120; Dead Space 2
(32-bit, at 3440x1440). It costs about 0.6 ms of the graphics card per game frame at 1080p. On Labyrinth OS it is in Agora
(Frame generation, preview): put `kairos %command%` in a game's Steam launch options once, then choose Off / 2x / 3x /
4x for that game in Settings > Games. How it looks on a real screen still needs eyes, and the second graphics card is still to
come. See [DESIGN.md](DESIGN.md).

```sh
cargo build --release
scripts/dev-layers.sh        # both layers and manifests pointing at the builds, in target/layers
VK_ADD_IMPLICIT_LAYER_PATH=$PWD/target/layers ENABLE_KAIROS=1 KAIROS_LOG=1 vkcube
```

Licence: MPL-2.0 (`LICENSE`).

## Version

`VERSION` is Kairos's own version (1.0.0 since 2026-11-11, its first numbered release); its package is `kairos-git` `VERSION-<build number>`. How the numbers work and how to make a release: [Labyrinth_OS docs/VERSIONING.md](https://github.com/Artifex-Technologies/Labyrinth_OS/blob/main/docs/VERSIONING.md).

## Artifex Ecosystem

| Repository | What it is |
| --- | --- |
| [Labyrinth_OS](https://github.com/Artifex-Technologies/Labyrinth_OS) | privacy- and security-first Linux distribution (CachyOS-based), made for gaming too |
| [LykanOS](https://github.com/Artifex-Technologies/LykanOS) | the security edition, built on the same desktop and tools |
| [Yggdrasil](https://github.com/Artifex-Technologies/Yggdrasil) | the desktop environment both systems use (panel, Settings, Agora, Minos, the compositor) |
| [Sarina](https://github.com/Artifex-Technologies/Sarina) | optional local AI assistant that runs on this computer |
| [Heimdall](https://github.com/Artifex-Technologies/Heimdall) | the security agent: watches the computer, its network and Sarina |
| [Limbo](https://github.com/Artifex-Technologies/Limbo) | disposable sandboxes for apps, Windows programs and untrusted files |
| [Kairos](https://github.com/Artifex-Technologies/Kairos) | **this repository**: frame generation for games (a Vulkan layer) |
| [Argus_App](https://github.com/Artifex-Technologies/Argus_App) | desktop console for the Flipper Zero (a qFlipper fork) |
| [Argus_Firmware](https://github.com/Artifex-Technologies/Argus_Firmware) | Flipper Zero firmware fork |
| [Daedalus_CAD](https://github.com/Artifex-Technologies/Daedalus_CAD) | 3D modelling and CAD (SketchUp-style) |
| [Icarus](https://github.com/Artifex-Technologies/Icarus) | vehicle diagnostics and tuning |
