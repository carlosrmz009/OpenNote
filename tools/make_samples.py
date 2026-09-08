"""Generate the sample scores in `samples/`.

Hand-writing MusicXML is tedious and easy to get subtly wrong, so the samples are
generated from a compact note list instead. Everything here is public domain.

Run with:  python tools/make_samples.py
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field

DIVISIONS = 4  # per quarter note

STEP_ALTER = {
    "C": ("C", 0), "C#": ("C", 1), "Db": ("D", -1),
    "D": ("D", 0), "D#": ("D", 1), "Eb": ("E", -1),
    "E": ("E", 0),
    "F": ("F", 0), "F#": ("F", 1), "Gb": ("G", -1),
    "G": ("G", 0), "G#": ("G", 1), "Ab": ("A", -1),
    "A": ("A", 0), "A#": ("A", 1), "Bb": ("B", -1),
    "B": ("B", 0),
}

TYPE_BY_DIVISIONS = {
    1: "16th", 2: "eighth", 3: "eighth", 4: "quarter",
    6: "quarter", 8: "half", 12: "half", 16: "whole",
}


@dataclass
class Note:
    """One note: a pitch name with octave, a duration in divisions, and a staff."""
    pitch: str | None  # None means a rest
    octave: int
    duration: int
    staff: int
    chord: bool = False


@dataclass
class Measure:
    """A bar, as two independent streams of notes."""
    upper: list[Note] = field(default_factory=list)
    lower: list[Note] = field(default_factory=list)


def note_xml(note: Note, voice: int) -> str:
    dots = ""
    kind = TYPE_BY_DIVISIONS.get(note.duration, "quarter")
    if note.duration in (3, 6, 12):
        dots = "<dot/>"
    if note.pitch is None:
        body = "<rest/>"
    else:
        step, alter = STEP_ALTER[note.pitch]
        alter_xml = f"<alter>{alter}</alter>" if alter else ""
        body = f"<pitch><step>{step}</step>{alter_xml}<octave>{note.octave}</octave></pitch>"
    chord = "<chord/>" if note.chord else ""
    return (
        f"      <note>{chord}{body}<duration>{note.duration}</duration>"
        f"<voice>{voice}</voice><type>{kind}</type>{dots}<staff>{note.staff}</staff></note>\n"
    )


def build(title: str, beats: int, beat_type: int, fifths: int, tempo: int,
          measures: list[Measure]) -> str:
    out = [
        '<?xml version="1.0" encoding="UTF-8"?>\n',
        '<!DOCTYPE score-partwise PUBLIC "-//Recordare//DTD MusicXML 4.0 Partwise//EN"'
        ' "http://www.musicxml.org/dtds/partwise.dtd">\n',
        '<score-partwise version="4.0">\n',
        f"  <work><work-title>{title}</work-title></work>\n",
        '  <part-list><score-part id="P1"><part-name>Piano</part-name></score-part></part-list>\n',
        '  <part id="P1">\n',
    ]
    for index, measure in enumerate(measures, start=1):
        out.append(f'    <measure number="{index}">\n')
        if index == 1:
            out.append(
                "      <attributes>\n"
                f"        <divisions>{DIVISIONS}</divisions>\n"
                f"        <key><fifths>{fifths}</fifths></key>\n"
                f"        <time><beats>{beats}</beats><beat-type>{beat_type}</beat-type></time>\n"
                "        <staves>2</staves>\n"
                '        <clef number="1"><sign>G</sign><line>2</line></clef>\n'
                '        <clef number="2"><sign>F</sign><line>4</line></clef>\n'
                "      </attributes>\n"
                '      <direction placement="above"><direction-type>'
                f'<metronome><beat-unit>quarter</beat-unit><per-minute>{tempo}</per-minute></metronome>'
                f'</direction-type><sound tempo="{tempo}"/></direction>\n'
            )
        for note in measure.upper:
            out.append(note_xml(note, 1))
        total_upper = sum(n.duration for n in measure.upper if not n.chord)
        if measure.lower:
            if total_upper:
                out.append(f"      <backup><duration>{total_upper}</duration></backup>\n")
            for note in measure.lower:
                out.append(note_xml(note, 5))
        out.append("    </measure>\n")
    out.append("  </part>\n</score-partwise>\n")
    return "".join(out)


def n(pitch, octave, duration, staff=1, chord=False):
    return Note(pitch, octave, duration, staff, chord)


def rest(duration, staff=1):
    return Note(None, 0, duration, staff)


def fur_elise() -> str:
    """Beethoven, Bagatelle in A minor WoO 59, opening. 3/8, quaver = 1 beat."""
    m = []
    # Pickup bar: two semiquavers.
    m.append(Measure(upper=[n("E", 5, 1), n("D#", 5, 1)], lower=[rest(2, 2)]))
    # E D# E B D C
    m.append(Measure(
        upper=[n("E", 5, 1), n("D#", 5, 1), n("E", 5, 1), n("B", 4, 1), n("D", 5, 1), n("C", 5, 1)],
        lower=[rest(6, 2)],
    ))
    # A, then the left hand's broken A minor chord.
    m.append(Measure(
        upper=[n("A", 4, 2), rest(1), n("C", 4, 1), n("E", 4, 1), n("A", 4, 1)],
        lower=[n("A", 2, 2, 2), n("E", 3, 2, 2), n("A", 3, 2, 2)],
    ))
    # B, then E major.
    m.append(Measure(
        upper=[n("B", 4, 2), rest(1), n("E", 4, 1), n("G#", 4, 1), n("B", 4, 1)],
        lower=[n("E", 2, 2, 2), n("E", 3, 2, 2), n("G#", 3, 2, 2)],
    ))
    # C, then back to A minor, and the theme returns.
    m.append(Measure(
        upper=[n("C", 5, 2), rest(1), n("E", 4, 1), n("E", 5, 1), n("D#", 5, 1)],
        lower=[n("A", 2, 2, 2), n("E", 3, 2, 2), n("A", 3, 2, 2)],
    ))
    return build("Fur Elise (opening)", 3, 8, 0, 60, m)


def scales_and_chords() -> str:
    """A two-octave C major scale in both hands, then triads. A calibration piece:
    every fingering in it is one that every method book agrees on."""
    m = []
    scale_up = [("C", 4), ("D", 4), ("E", 4), ("F", 4), ("G", 4), ("A", 4), ("B", 4), ("C", 5)]
    scale_up += [("D", 5), ("E", 5), ("F", 5), ("G", 5), ("A", 5), ("B", 5), ("C", 6)]
    low = [(p, o - 2) for p, o in scale_up]

    # Four bars of 4/4, four quavers... the scale is 15 notes, so lay it out as
    # semiquavers, four to a beat.
    upper_notes = [n(p, o, 2) for p, o in scale_up]
    lower_notes = [n(p, o, 2, 2) for p, o in low]
    for start in range(0, 16, 8):
        chunk_u = upper_notes[start:start + 8]
        chunk_l = lower_notes[start:start + 8]
        while sum(x.duration for x in chunk_u) < 16:
            chunk_u.append(rest(16 - sum(x.duration for x in chunk_u)))
        while sum(x.duration for x in chunk_l) < 16:
            chunk_l.append(rest(16 - sum(x.duration for x in chunk_l), 2))
        m.append(Measure(upper=chunk_u, lower=chunk_l))

    # Triads: C, F, G, C.
    for root_upper, root_lower in [
        (("C", 4, "E", 4, "G", 4), ("C", 3, "E", 3, "G", 3)),
        (("C", 4, "F", 4, "A", 4), ("F", 2, "A", 2, "C", 3)),
        (("B", 3, "D", 4, "G", 4), ("G", 2, "B", 2, "D", 3)),
        (("C", 4, "E", 4, "G", 4), ("C", 3, "E", 3, "G", 3)),
    ]:
        up = [
            n(root_upper[0], root_upper[1], 16),
            n(root_upper[2], root_upper[3], 16, chord=True),
            n(root_upper[4], root_upper[5], 16, chord=True),
        ]
        lo = [
            n(root_lower[0], root_lower[1], 16, 2),
            n(root_lower[2], root_lower[3], 16, 2, chord=True),
            n(root_lower[4], root_lower[5], 16, 2, chord=True),
        ]
        m.append(Measure(upper=up, lower=lo))

    return build("Scales and chords", 4, 4, 0, 72, m)


def main() -> None:
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
    out = os.path.join(root, "samples")
    os.makedirs(out, exist_ok=True)
    for name, text in [
        ("fur-elise.musicxml", fur_elise()),
        ("scales-and-chords.musicxml", scales_and_chords()),
    ]:
        path = os.path.join(out, name)
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(text)
        print("wrote", os.path.relpath(path, root))


if __name__ == "__main__":
    main()
