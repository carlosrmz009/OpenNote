#!/bin/sh
# Keep improving every part of the engine, for as long as this is left running.
#
# Each target is searched for a few hours and then the next one, round and round.
# Everything a search needs to carry on is written after every generation, so the
# handover costs nothing but the minute it takes to read the scores back in, and the
# whole thing can be stopped at any moment — Ctrl+C, a power cut — and started again.
#
#   ./tools/tune_forever.sh ~/PDMX/PDMX/data
#
# The two targets are given different numbers because a score costs about three hundred
# times as much to finger as it does to divide between the hands. The hand target reads
# whole pieces and judges a generation on six hundred of them; the playing target reads
# a passage from each and judges on two hundred. Anything passed after the folder is
# added to both, and overrides what is set here.
#
# On a Raspberry Pi — four cores, each about ten times slower — pass
# `--batch 150 --notes 80 --limit 6000` to keep a generation near a minute and the whole
# thing inside a gigabyte.
set -e
scores=${1:?usage: tune_forever.sh <PDMX/data folder> [extra opennote arguments...]}
shift
opennote=${OPENNOTE:-./target/release/opennote}
while true; do
    "$opennote" tune --target hands --scores "$scores" --hours 4 "$@"
    "$opennote" tune --target play --scores "$scores" --hours 4 \
        --limit 20000 --notes 120 --batch 200 "$@"
done
