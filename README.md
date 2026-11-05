# labyrinth-framegen

Frame generation for Labyrinth OS, as a Vulkan layer: more frames on the screen than the game draws, for native Vulkan
games and Proton games alike. Our own: it needs no other app, and it is meant to run the generation on a second graphics
card too. Working name.

**State (2026-11-05): an early prototype.** The layer adds frames at the right moments (vkcube: 90 game frames a second,
180 on the screen), but the added frames are still copies; real frames in between are the next milestone. See
[DESIGN.md](DESIGN.md).

```sh
cargo build --release
sed 's#"liblabyrinth#"./liblabyrinth#' data/labyrinth_framegen.json > target/release/labyrinth_framegen.json
VK_ADD_IMPLICIT_LAYER_PATH=$PWD/target/release ENABLE_LABYRINTH_FRAMEGEN=1 LABYRINTH_FRAMEGEN_LOG=1 vkcube
```

Licence: MPL-2.0 (`LICENSE`).
