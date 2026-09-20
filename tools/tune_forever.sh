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
# On a Raspberry Pi, pass a smaller batch: a Pi is about ten times slower per core and
# has four of them, so `--batch 150 --notes 80` keeps a generation to about a minute.
# Memory is the other limit; fifteen thousand scores need about a gigabyte, and
# `--limit 6000` brings that down if the board has less.
set -e
scores=${1:?usage: tune_forever.sh <PDMX/data folder> [extra opennote arguments...]}
shift
opennote=${OPENNOTE:-./target/release/opennote}
while true; do
    "$opennote" tune --target hands --scores "$scores" --hours 4 "$@"
    "$opennote" tune --target play --scores "$scores" --hours 4 "$@"
done
