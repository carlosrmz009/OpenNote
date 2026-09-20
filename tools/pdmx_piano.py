"""Pull the piano scores out of the PDMX archive.

PDMX (https://github.com/pnlong/PDMX, CC BY 4.0) ships as one PDMX.tar.gz of about
2.4 GB. Its scores are JSON in its authors' own format, which keeps one track per staff.
Every one of its 254,077 scores is public domain (210,364) or CC0 (43,713) — the
dataset's authors filtered for that — so nothing here turns on the licence. The format
has no fingerings, so none of it is any use for `opennote tune --target fingers`.

This keeps the scores where every track is a piano (instrument 0), which is 51,774 of
them once the two filters below are applied. Of those, 14,936 have exactly two piano
tracks, which is a two-staff score and so a score whose source says which hand plays
what; that is what `--target hands` needs, and it takes them out of this set itself.
The other 36,838 are one staff, or more than two, and `--target play` uses those as
well — it does not need to know which hand played what, only whether what the engine
writes can be played.

The two filters are quality, not licence:

* in the de-duplicated subset, so one hymn uploaded forty times is not forty examples;
* at least 100 notes long.

One more exists and is deliberately not applied. For about 12% of PDMX, what
MuseScore's page says about the copyright and what the file itself says disagree, and
the release at https://zenodo.org/records/15571083 flags those as a licence conflict.
Applying it would leave 2,241 two-staff scores of the 14,936. Since every score in the
dataset is claimed public domain or CC0 either way, and since nothing is redistributed
from them — what a search takes out of a score is a number — the conflict flag is
recorded here rather than enforced.

    python tools/pdmx_piano.py path/to/PDMX.tar.gz

It writes the list of what it kept to PDMX/piano.txt next to the archive.
"""

import csv
import io
import sys
import tarfile
from pathlib import Path


def main() -> None:
    archive = Path(sys.argv[1] if len(sys.argv) > 1 else "PDMX.tar.gz")
    out = archive.parent
    with tarfile.open(archive, "r:gz") as tar:
        table = tar.extractfile("PDMX/PDMX.csv")
        rows = csv.DictReader(io.TextIOWrapper(table, encoding="utf-8"))
        keep = {
            "PDMX/" + row["path"][2:]
            for row in rows
            if all(track == "0" for track in row["tracks"].split("-"))
            and row["subset:deduplicated"] == "True"
            and int(row["n_notes"]) >= 100
        }
        print(f"{len(keep)} piano scores; extracting...")
        # Streamed in one pass: the archive is gzip, so asking for members by name
        # would read it from the start again for every one.
        with tarfile.open(archive, "r|gz") as stream:
            for member in stream:
                if member.name in keep:
                    stream.extract(member, out, filter="data")
    listing = out / "PDMX" / "piano.txt"
    listing.write_text("\n".join(sorted(keep)) + "\n", encoding="utf-8", newline="\n")
    print(f"done; the scores are under {out / 'PDMX' / 'data'}")


if __name__ == "__main__":
    main()
