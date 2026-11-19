# Contracts with other repositories

What other projects of the Artifex Ecosystem rely on from Kairos. One section per contract: what, who uses it, where in the
code. A change to any of them says `CONTRACT:` in the commit message, changes this file in the same commit, and is told to
the other repository's maintainers.

## The per-game settings file: `~/.config/kairos/games.conf`

- **What:** one line per game, `<Steam app id> <off|2|3|4>`, and `default <off|2|3|4>` for the rest. The launcher reads the
  old `~/.config/labyrinth-framegen/games.conf` while the new file does not exist.
- **Who:** Yggdrasil Settings > Games writes it (`packages/settings/src/games.rs`).
- **Where:** `bin/kairos` (the launcher); `tests/launcher.sh` checks it.

## The launcher: `kairos %command%`

- **What:** `/usr/bin/kairos` in a game's Steam launch options reads the game's id (SteamAppId, SteamGameId or
  STEAM_COMPAT_APP_ID) and starts the game with the layer on at the chosen multiplier, or untouched.
  `/usr/bin/labyrinth-framegen` points at it, so launch options written before the rename still work.
- **Who:** Yggdrasil Settings > Games (it tells people to add the launch option), Labyrinth OS's package `kairos-git`.
- **Where:** `bin/kairos`.

## The layer and its variables

- **What:** the implicit Vulkan layers `VK_LAYER_KAIROS_x86_64` and `VK_LAYER_KAIROS_x86` (manifests in
  `/usr/share/vulkan/implicit_layer.d`, libraries in `/usr/lib` and `/usr/lib32`), loaded only with `ENABLE_KAIROS=1`.
  Settings: `KAIROS_MULTIPLIER` (2 to 4), `KAIROS_LOG=1`, `KAIROS_LOG_FILE=path`, `KAIROS_MODE=copy`, `KAIROS_DEBUG=flow|mark`;
  the old `LABYRINTH_FRAMEGEN_*` names are still read.
- **Who:** the launcher; Labyrinth OS's `kairos-git` recipe (installs the files); Limbo's `limbo wine` sandbox (the system's
  Vulkan layers, Kairos included, are visible to programs in it).
- **Where:** `data/kairos.x86_64.json`, `data/kairos.x86.json`, `src/lib.rs`.
