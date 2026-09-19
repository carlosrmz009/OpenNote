"""Pull the two-staff piano scores out of the PDMX archive.

PDMX (https://github.com/pnlong/PDMX, CC BY 4.0) ships as one PDMX.tar.gz of about
2.4 GB. Its scores are JSON in its authors' own format, which keeps one track per
staff: a two-staff piano score is two piano tracks, and that is what
`opennote tune --target hands` learns from. The format has no fingerings, so it is no
use for `--target fingers`.

This keeps the scores that are:

* exactly two piano tracks (`tracks == "0-0"`);
* in the de-duplicated subset, so one hymn uploaded forty times is not forty examples;
* public domain or CC0, so what is learned from them can ship;
* at least 100 notes long.

and extracts only those, which is about 15,000 files and 1.4 GB.

    python tools/pdmx_piano.py path/to/PDMX.tar.gz

It writes the list of what it kept to PDMX/piano_two_staff.txt next to the archive.
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
            if row["tracks"] == "0-0"
            and row["subset:deduplicated"] == "True"
            and row["license"] in ("publicdomain", "cc-zero")
            and int(row["n_notes"]) >= 100
        }
        print(f"{len(keep)} two-staff piano scores; extracting...")
        # Streamed in one pass: the archive is gzip, so asking for members by name
        # would read it from the start again for every one.
        with tarfile.open(archive, "r|gz") as stream:
            for member in stream:
                if member.name in keep:
                    stream.extract(member, out, filter="data")
    listing = out / "PDMX" / "piano_two_staff.txt"
    listing.write_text("\n".join(sorted(keep)) + "\n", encoding="utf-8", newline="\n")
    print(f"done; the scores are under {out / 'PDMX' / 'data'}")


if __name__ == "__main__":
    main()
