#!/bin/sh
# The launcher's choices: per game, "off", the default, and no settings at all.
set -eu
L="$(dirname "$0")/../bin/labyrinth-framegen"
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
mkdir -p "$T/labyrinth-framegen"
printf '47780 3\n1332010 off\ndefault 2\n' > "$T/labyrinth-framegen/games.conf"
run() { SteamAppId="$1" XDG_CONFIG_HOME="$T" "$L" sh -c 'echo "${ENABLE_LABYRINTH_FRAMEGEN:-no}:${LABYRINTH_FRAMEGEN_MULTIPLIER:--}"'; }
[ "$(run 47780)" = "1:3" ]
[ "$(run 1332010)" = "no:-" ]
[ "$(run 999)" = "1:2" ]
rm "$T/labyrinth-framegen/games.conf"
[ "$(run 47780)" = "no:-" ]
echo "launcher: ok"
