import argparse
import pathlib
import shutil
import subprocess
import sys
import tarfile
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
SOURCE = "https://downloads.xiph.org/releases/speex/speex-1.2.1.tar.gz"
FLAGS = ["-O2", "-ffp-contract=off", "-fno-fast-math", "-fno-tree-vectorize"]
BANDS = {"nb": (8000, 160), "wb": (16000, 320), "uwb": (32000, 640)}
LOST = b"\xff\xff"
KEPT = [
    ("nb_q0", 30), ("nb_q3", 30), ("nb_q6", 30), ("nb_q8", 30), ("nb_q10", 30),
    ("wb_q0", 30), ("wb_q4", 30), ("wb_q6", 30), ("wb_q10", 30),
    ("uwb_q1", 25), ("uwb_q5", 25), ("uwb_q10", 25),
    ("nb_vbr8", 30), ("wb_vbr8", 30), ("uwb_vbr8", 20),
    ("nb_vbr3_quiet", 100), ("wb_vbr3_quiet", 40),
    ("nb_q1", 30), ("nb_q2", 30),
    ("nb_q4_x2", 15), ("nb_q4_x5", 8), ("nb_q6_x3", 10), ("nb_q2_x2", 15), ("wb_q6_x3", 8), ("uwb_q5_x2", 8),
    ("nb_q6_loss", 60), ("wb_q6_loss", 50), ("uwb_q6_loss", 30),
    ("nb_q5_quiet", 40),
]
KEPT_PLAIN = [("nb_q6", 30), ("wb_q6", 30)]


def run(command, cwd=None, quiet=False):
    done = subprocess.run(command, cwd=cwd, capture_output=quiet, text=True)
    if done.returncode != 0:
        if quiet:
            sys.stderr.write((done.stdout or "")[-2000:] + (done.stderr or "")[-2000:])
        sys.exit(f"{command[0]} failed with exit code {done.returncode}")


def build(work):
    work.mkdir(parents=True, exist_ok=True)
    archive = work / "speex-1.2.1.tar.gz"
    tree = work / "speex-1.2.1"
    library = tree / "libspeex" / ".libs" / "libspeex.a"
    if not archive.is_file():
        print("fetching", SOURCE)
        urllib.request.urlretrieve(SOURCE, archive)
    if not library.is_file():
        shutil.rmtree(tree, ignore_errors=True)
        with tarfile.open(archive) as packed:
            packed.extractall(work)
        configure = ["./configure", "--disable-shared", "--enable-static", "--disable-sse", "--disable-binaries"]
        run(configure + ["CFLAGS=" + " ".join(FLAGS)], cwd=tree, quiet=True)
        run(["make", "-j4"], cwd=tree, quiet=True)
    tool = work / "speexref"
    run(["gcc", *FLAGS, "-I", str(tree / "include"), str(HERE / "speexref.c"), str(library), "-lm", "-o", str(tool)])
    print("built", tool)
    return tool


def speech(numpy, rate, seconds, seed):
    n = int(rate * seconds)
    t = numpy.arange(n) / rate
    rng = numpy.random.default_rng(seed)
    pitch = 125 + 35 * numpy.sin(2 * numpy.pi * 0.7 * t) + 12 * numpy.sin(2 * numpy.pi * 2.3 * t + 1.0)
    phase = 2 * numpy.pi * numpy.cumsum(pitch) / rate
    formants = [
        (600 + 250 * numpy.sin(2 * numpy.pi * 1.1 * t), 90.0),
        (1500 + 500 * numpy.sin(2 * numpy.pi * 0.8 * t + 0.6), 130.0),
        (2600 + 300 * numpy.sin(2 * numpy.pi * 0.5 * t + 1.9), 180.0),
    ]
    voiced = numpy.zeros(n)
    limit = 0.45 * rate
    for k in range(1, 80):
        freq = k * pitch
        if freq.min() >= limit:
            break
        weight = numpy.zeros(n)
        for centre, width in formants:
            weight += 1.0 / (1.0 + ((freq - centre) / width) ** 2)
        weight *= freq < limit
        voiced += weight * numpy.sin(k * phase) / numpy.sqrt(k)
    syllable = numpy.clip(numpy.sin(2 * numpy.pi * 3.7 * t) * 1.6 + 0.5, 0.0, 1.0) ** 1.5
    pause = ((t % 1.3) < 1.0).astype(float)
    edge = numpy.convolve(pause, numpy.ones(int(rate * 0.01)) / int(rate * 0.01), mode="same")
    noise = rng.standard_normal(n)
    hiss = numpy.concatenate(([0.0], numpy.diff(noise)))
    fricative = numpy.clip(numpy.sin(2 * numpy.pi * 1.9 * t + 2.0) - 0.75, 0.0, 1.0) * 4.0
    signal = voiced * syllable * edge + 0.25 * hiss * fricative * edge + 0.002 * noise
    signal *= 0.1 / (numpy.sqrt(numpy.mean(signal**2)) + 1e-12)
    return numpy.clip(signal, -0.95, 0.95)


def quiet(numpy, rate, seconds, seed):
    n = int(rate * seconds)
    rng = numpy.random.default_rng(seed)
    signal = numpy.zeros(n)
    half = n // 2
    signal[half:] = 0.0015 * rng.standard_normal(n - half)
    tone_from = int(n * 0.8)
    signal[tone_from:] += 0.2 * numpy.sin(2 * numpy.pi * 1000 * numpy.arange(n - tone_from) / rate)
    return signal


def signals(work):
    try:
        import numpy
    except ImportError:
        sys.exit("the test sounds need numpy; run this step with a Python that has it")
    sounds = work / "in"
    sounds.mkdir(parents=True, exist_ok=True)
    for tag, (rate, _) in BANDS.items():
        for name, signal in (("speech", speech(numpy, rate, 6.0, 7)), ("quiet", quiet(numpy, rate, 6.0, 17))):
            data = numpy.round(signal * 32767.0).astype("<i2")
            (sounds / f"{name}_{tag}.s16").write_bytes(data.tobytes())
    print("test sounds written to", sounds)


def records(path):
    data = path.read_bytes()
    out = []
    at = 0
    while at + 2 <= len(data):
        length = data[at] | (data[at + 1] << 8)
        at += 2
        if length == 0xFFFF:
            out.append(None)
        else:
            out.append(data[at:at + length])
            at += length
    return out


def packed(packets):
    out = bytearray()
    for packet in packets:
        out += LOST if packet is None else bytes((len(packet) & 0xFF, len(packet) >> 8)) + packet
    return bytes(out)


def streams(work):
    tool = work / "speexref"
    sounds = work / "in"
    out = work / "speex"
    out.mkdir(parents=True, exist_ok=True)

    def encode(band, quality, vbr, per_packet, sound, name):
        run([str(tool), "enc", band, str(quality), str(vbr), str(per_packet), str(sounds / f"{sound}_{band}.s16"), str(out / f"{name}.pkt")], quiet=True)

    def decode(band, enhance, name, sound_name=None):
        run([str(tool), "dec", band, str(enhance), str(out / f"{name}.pkt"), str(out / f"{sound_name or name}.f32")], quiet=True)

    for band in BANDS:
        for quality in range(11):
            encode(band, quality, 0, 1, "speech", f"{band}_q{quality}")
            decode(band, 1, f"{band}_q{quality}")
        decode(band, 0, f"{band}_q6", f"{band}_q6_plain")
        encode(band, 8, 1, 1, "speech", f"{band}_vbr8")
        decode(band, 1, f"{band}_vbr8")
        encode(band, 3, 1, 1, "quiet", f"{band}_vbr3_quiet")
        decode(band, 1, f"{band}_vbr3_quiet")
        encode(band, 5, 0, 1, "quiet", f"{band}_q5_quiet")
        decode(band, 1, f"{band}_q5_quiet")
        for quality, per_packet in ((6, 3), (2, 2), (5, 2), (4, 2), (4, 5)):
            encode(band, quality, 0, per_packet, "speech", f"{band}_q{quality}_x{per_packet}")
            decode(band, 1, f"{band}_q{quality}_x{per_packet}")
        whole = records(out / f"{band}_q6.pkt")
        thinned = [
            None if index % 11 == 7 or 20 <= index < 23 or index in (45, 46) or 90 <= index < 96 else packet
            for index, packet in enumerate(whole)
        ]
        (out / f"{band}_q6_loss.pkt").write_bytes(packed(thinned))
        decode(band, 1, f"{band}_q6_loss")
    print(len(list(out.glob("*.pkt"))), "streams in", out)


def frames_in(name):
    for mark, count in (("_x2", 2), ("_x3", 3), ("_x5", 5)):
        if mark in name:
            return count
    return 1


def pack(work, target):
    try:
        import numpy
    except ImportError:
        sys.exit("packing needs numpy; run this step with a Python that has it")
    source = work / "speex"
    target.mkdir(parents=True, exist_ok=True)
    total = 0

    def expected(sound_name, samples):
        sound = numpy.fromfile(source / f"{sound_name}.f32", dtype="<f4")[:samples]
        if len(sound) != samples:
            sys.exit(f"{sound_name}: only {len(sound)} samples, wanted {samples}")
        return numpy.clip(numpy.round(sound), -32768, 32767).astype("<i2").tobytes()

    for name, keep in KEPT:
        band = name.split("_")[0]
        stream = packed(records(source / f"{name}.pkt")[:keep])
        sound = expected(name, keep * frames_in(name) * BANDS[band][1])
        (target / f"{name}.pkt").write_bytes(stream)
        (target / f"{name}.s16").write_bytes(sound)
        total += len(stream) + len(sound)
    for name, keep in KEPT_PLAIN:
        band = name.split("_")[0]
        sound = expected(f"{name}_plain", keep * BANDS[band][1])
        (target / f"{name}_plain.s16").write_bytes(sound)
        total += len(sound)
    print(f"{len(KEPT)} streams packed into {target}, {total // 1024} KiB")


def full(work, target):
    source = work / "speex"
    target.mkdir(parents=True, exist_ok=True)
    count = 0
    for path in sorted(source.glob("*")):
        if path.suffix in (".pkt", ".f32"):
            shutil.copyfile(path, target / path.name)
            count += 1
    print(f"{count} files copied to {target}")


def main():
    parser = argparse.ArgumentParser(
        description="Make the Speex reference streams the ps-oldcodecs tests compare against, from libspeex 1.2.1."
    )
    parser.add_argument("step", choices=["build", "signals", "streams", "pack", "full"])
    parser.add_argument("--work", required=True, help="a scratch folder; the same one for every step")
    args = parser.parse_args()
    work = pathlib.Path(args.work).resolve()
    data = ROOT / "ps-oldcodecs" / "tests"
    if args.step == "build":
        build(work)
    elif args.step == "signals":
        signals(work)
    elif args.step == "streams":
        streams(work)
    elif args.step == "pack":
        pack(work, data / "data" / "speex")
    else:
        full(work, data / "full" / "speex")


main()
