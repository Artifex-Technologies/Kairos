#!/bin/sh
# The launcher's choices: per game, "off", the default, no settings at all, and the settings file's name before the rename.
set -eu
L="$(dirname "$0")/../bin/kairos"
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
mkdir -p "$T/kairos" "$T/labyrinth-framegen"
printf '47780 3\n1332010 off\ndefault 2\n' > "$T/kairos/games.conf"
run() { SteamAppId="$1" XDG_CONFIG_HOME="$T" "$L" sh -c 'echo "${ENABLE_KAIROS:-no}:${KAIROS_MULTIPLIER:--}"'; }
[ "$(run 47780)" = "1:3" ]
[ "$(run 1332010)" = "no:-" ]
[ "$(run 999)" = "1:2" ]
rm "$T/kairos/games.conf"
[ "$(run 47780)" = "no:-" ]
# only the old file: still read
printf '47780 4\n' > "$T/labyrinth-framegen/games.conf"
[ "$(run 47780)" = "1:4" ]
echo "launcher: ok"
