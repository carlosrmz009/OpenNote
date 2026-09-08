"""Watch a pianist play, and write down where their fingers were.

There is a great deal of piano on the internet filmed from directly overhead, and
every one of those videos is somebody showing you their fingering. This turns one into
training data for `opennote train`.

Two steps, and the first only has to be done once per camera position — which in
practice means once per channel, since a given pianist films from the same place every
time.

    # 1. Say where the keyboard is in the frame. Click four corners.
    python tools/watch_pianist.py calibrate video.mp4 --out corners.json
    opennote corpus calibrate corners.json --out camera.json

    # 2. Find the hands, frame by frame.
    python tools/watch_pianist.py watch video.mp4 --out video.landmarks.json

    # 3. Join the two to the notes, and put the result in the corpus.
    opennote corpus watch video.landmarks.json \\
        --score video.mid --calibration camera.json --annotator rousseau

The method is PianoMime's (Qian, Urain, Zakka & Peters, 2024). Only the part that
needs a neural network is here; deciding which finger played which key is done in
Rust, in `on-train`, where it is tested.

Needs `opencv-python` and `mediapipe`, and MediaPipe's hand landmarker model:

    pip install opencv-python mediapipe
    curl -O https://storage.googleapis.com/mediapipe-models/hand_landmarker/hand_landmarker/float16/1/hand_landmarker.task
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

try:
    import cv2
except ImportError:  # pragma: no cover - a helper script, not part of the build
    print("This needs OpenCV:  pip install opencv-python", file=sys.stderr)
    raise SystemExit(2)

# MediaPipe's hand model numbers twenty-one landmarks per hand. These five are the
# fingertips, thumb first; nothing downstream needs to know the rest of the numbering.
FINGERTIPS = (4, 8, 12, 16, 20)
WRIST = 0

# Where the four calibration clicks go, in the order they are asked for.
CORNERS = (
    "the FAR-LEFT corner of the keyboard (top left of the white keys)",
    "the FAR-RIGHT corner (top right of the white keys)",
    "the NEAR-RIGHT corner (bottom right, nearest the player)",
    "the NEAR-LEFT corner (bottom left)",
)

# Middle C is 60; the notes are named so the prompt can ask for one.
NAMES = ("C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B")


def note_name(midi: int) -> str:
    return f"{NAMES[midi % 12]}{midi // 12 - 1}"


def first_frame(path: Path):
    """The first frame of a video, for clicking on."""
    capture = cv2.VideoCapture(str(path))
    if not capture.isOpened():
        raise SystemExit(f"could not open {path}")
    # A little way in, so a title card or a fade is not what gets calibrated.
    capture.set(cv2.CAP_PROP_POS_FRAMES, capture.get(cv2.CAP_PROP_FRAME_COUNT) // 10)
    ok, frame = capture.read()
    capture.release()
    if not ok:
        raise SystemExit(f"could not read a frame from {path}")
    return frame


def calibrate(args: argparse.Namespace) -> None:
    """Click the keyboard's four corners, and say which keys they are."""
    frame = first_frame(Path(args.video))
    clicks: list[tuple[int, int]] = []

    def on_click(event, x, y, _flags, _params):
        if event != cv2.EVENT_LBUTTONDOWN or len(clicks) >= 4:
            return
        clicks.append((x, y))
        cv2.circle(frame, (x, y), 6, (0, 220, 255), -1)
        cv2.imshow("calibrate", frame)

    print("Click, in order:")
    for index, corner in enumerate(CORNERS):
        print(f"  {index + 1}. {corner}")
    print("Then press any key.")

    cv2.imshow("calibrate", frame)
    cv2.setMouseCallback("calibrate", on_click)
    while len(clicks) < 4:
        if cv2.waitKey(20) != -1:
            break
    cv2.destroyAllWindows()
    if len(clicks) < 4:
        raise SystemExit("four corners are needed")

    lowest = args.lowest
    highest = args.highest
    if lowest is None:
        lowest = int(input(f"MIDI number of the leftmost white key [{21}]: ") or 21)
    if highest is None:
        highest = int(input(f"MIDI number of the rightmost white key [{108}]: ") or 108)
    print(f"Leftmost {note_name(lowest)}, rightmost {note_name(highest)}.")

    # The clicks are all this writes. Fitting the homography to them is arithmetic
    # the program already does — and does with tests around it — so it is left there
    # rather than reimplemented here where it could quietly disagree.
    payload = {
        "corners": [list(point) for point in clicks],
        "lowest_white": lowest,
        "highest_white": highest,
    }
    Path(args.out).write_text(json.dumps(payload, indent=2))
    print(f"Wrote {args.out}.")
    print(f"\nNext:  opennote corpus calibrate {args.out} --out camera.json")


def watch(args: argparse.Namespace) -> None:
    """Run the hand tracker over a video and write down what it found."""
    try:
        import mediapipe as mp
        from mediapipe.tasks import python as mp_python
        from mediapipe.tasks.python import vision
    except ImportError:
        print("This needs MediaPipe:  pip install mediapipe", file=sys.stderr)
        raise SystemExit(2)

    model = Path(args.model)
    if not model.exists():
        raise SystemExit(
            f"{model} is missing. Download MediaPipe's hand landmarker:\n"
            "  curl -O https://storage.googleapis.com/mediapipe-models/hand_landmarker/"
            "hand_landmarker/float16/1/hand_landmarker.task"
        )

    detector = vision.HandLandmarker.create_from_options(
        vision.HandLandmarkerOptions(
            base_options=mp_python.BaseOptions(model_asset_path=str(model)),
            running_mode=vision.RunningMode.VIDEO,
            num_hands=2,
            # Piano hands are small in frame and often partly occluded by each other,
            # so the thresholds are looser than the defaults.
            min_hand_detection_confidence=0.3,
            min_tracking_confidence=0.3,
            min_hand_presence_confidence=0.3,
        )
    )

    capture = cv2.VideoCapture(args.video)
    if not capture.isOpened():
        raise SystemExit(f"could not open {args.video}")
    fps = capture.get(cv2.CAP_PROP_FPS) or 30.0
    total = int(capture.get(cv2.CAP_PROP_FRAME_COUNT))
    # No point sampling faster than the fingering changes; every other frame at 30 fps
    # is plenty and halves the work.
    stride = max(1, round(fps / args.rate))
    print(f"{args.video}: {total} frames at {fps:.1f} fps, sampling every {stride}")

    frames = []
    index = 0
    seen = 0
    while True:
        ok, image = capture.read()
        if not ok:
            break
        if index % stride:
            index += 1
            continue
        time = index / fps
        rgb = cv2.cvtColor(image, cv2.COLOR_BGR2RGB)
        picture = mp.Image(image_format=mp.ImageFormat.SRGB, data=rgb)
        result = detector.detect_for_video(picture, int(time * 1000))

        height, width = image.shape[:2]
        hands = []
        for landmarks, handedness in zip(result.hand_landmarks, result.handedness):
            def at(point):
                return [landmarks[point].x * width, landmarks[point].y * height]

            hands.append(
                {
                    # MediaPipe labels handedness as though looking at the person, so
                    # its "Left" is the pianist's left. Which hand is which is decided
                    # again downstream from where the wrists are, since that is never
                    # wrong for somebody sitting at a piano.
                    "hand": "left" if handedness[0].category_name == "Left" else "right",
                    "fingertips": [at(point) for point in FINGERTIPS],
                    "wrist": at(WRIST),
                }
            )
        if hands:
            seen += 1
        frames.append({"time": time, "hands": hands})
        index += 1

        if args.limit and len(frames) >= args.limit:
            break
        if len(frames) % 200 == 0:
            print(f"  {index}/{total} frames, hands in {seen} of {len(frames)} sampled")

    capture.release()
    payload = {"source": args.video, "offset": args.offset, "frames": frames}
    Path(args.out).write_text(json.dumps(payload))
    print(f"Wrote {args.out}: {len(frames)} frames, hands found in {seen}.")
    if seen < len(frames) // 2:
        print(
            "\nHands were found in fewer than half the frames. Either the video is not "
            "shot from overhead, or the hands are small in frame — try a closer crop."
        )


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    commands = parser.add_subparsers(dest="command", required=True)

    calibration = commands.add_parser("calibrate", help="say where the keyboard is")
    calibration.add_argument("video")
    calibration.add_argument("--out", default="corners.json")
    calibration.add_argument("--lowest", type=int, help="MIDI number of the leftmost white key")
    calibration.add_argument("--highest", type=int, help="MIDI number of the rightmost white key")
    calibration.set_defaults(run=calibrate)

    watcher = commands.add_parser("watch", help="find the hands in a video")
    watcher.add_argument("video")
    watcher.add_argument("--out", default="landmarks.json")
    watcher.add_argument("--model", default="hand_landmarker.task")
    watcher.add_argument(
        "--rate", type=float, default=15.0, help="frames per second to sample"
    )
    watcher.add_argument(
        "--offset",
        type=float,
        default=0.0,
        help="seconds to add to the video's clock to line it up with the score",
    )
    watcher.add_argument("--limit", type=int, help="stop after this many sampled frames")
    watcher.set_defaults(run=watch)

    args = parser.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()
