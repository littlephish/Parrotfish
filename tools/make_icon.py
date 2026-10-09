import argparse
import base64
import collections
import pathlib
import re
import struct
import sys
import zlib

import numpy

ROOT = pathlib.Path(__file__).resolve().parent.parent
SIZES = (256, 64, 48, 40, 32, 24, 20, 16)
PNG_FROM = 256
SOLID = 8
FULLY_SOLID_FROM = 240.0
SIGNATURE = bytes([137, 80, 78, 71, 13, 10, 26, 10])


def fail(message):
    sys.exit(f"error: {message}")


def picture_bytes(path):
    data = path.read_bytes()
    if data.startswith(SIGNATURE):
        return data
    text = data.decode("utf-8", "replace")
    found = re.search(r'href="data:image/png;base64,([A-Za-z0-9+/=\s]+)"', text)
    if not found:
        fail(f"{path.name} is neither a PNG nor an SVG with a PNG inside")
    return base64.b64decode(found.group(1))


def read_png(data):
    if not data.startswith(SIGNATURE):
        fail("the picture is not a PNG")
    at = 8
    packed = bytearray()
    header = None
    while at + 8 <= len(data):
        length, kind = struct.unpack(">I4s", data[at:at + 8])
        body = data[at + 8:at + 8 + length]
        at += 12 + length
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            packed += body
        elif kind == b"IEND":
            break
    if header is None:
        fail("the PNG has no header")
    width, height, depth, colour, _, _, interlace = header
    if depth != 8 or colour not in (2, 6) or interlace != 0:
        fail("only plain 8-bit colour PNGs, with or without transparency, are read")
    channels = 4 if colour == 6 else 3
    raw = zlib.decompress(bytes(packed))
    stride = width * channels
    rows = numpy.frombuffer(raw, dtype=numpy.uint8).reshape(height, stride + 1)
    out = numpy.zeros((height, stride), dtype=numpy.uint8)
    before = numpy.zeros(stride, dtype=numpy.uint8)
    for y in range(height):
        kind = int(rows[y, 0])
        line = rows[y, 1:].astype(numpy.int32)
        if kind == 0:
            now = line
        elif kind == 2:
            now = line + before
        elif kind in (1, 3, 4):
            now = numpy.zeros(stride, dtype=numpy.int32)
            up = before.astype(numpy.int32)
            for x in range(stride):
                left = int(now[x - channels]) if x >= channels else 0
                above = int(up[x])
                corner = int(up[x - channels]) if x >= channels else 0
                if kind == 1:
                    guess = left
                elif kind == 3:
                    guess = (left + above) // 2
                else:
                    plain = left + above - corner
                    nearest = min((abs(plain - left), 0, left), (abs(plain - above), 1, above), (abs(plain - corner), 2, corner))
                    guess = nearest[2]
                now[x] = (int(line[x]) + guess) & 255
        else:
            fail(f"the PNG uses a row filter this script does not know ({kind})")
        out[y] = (now & 255).astype(numpy.uint8)
        before = out[y]
    pixels = out.reshape(height, width, channels)
    if channels == 3:
        pixels = numpy.dstack([pixels, numpy.full((height, width), 255, dtype=numpy.uint8)])
    return pixels


def write_png(pixels):
    height, width, _ = pixels.shape
    plain = pixels.reshape(height, width * 4).astype(numpy.int16)
    left = numpy.concatenate([numpy.zeros((height, 4), dtype=numpy.int16), plain[:, :-4]], axis=1)
    above = numpy.concatenate([numpy.zeros((1, width * 4), dtype=numpy.int16), plain[:-1]], axis=0)
    corner = numpy.concatenate([numpy.zeros((1, width * 4), dtype=numpy.int16), left[:-1]], axis=0)
    guess = left + above - corner
    by_left, by_above, by_corner = abs(guess - left), abs(guess - above), abs(guess - corner)
    nearest = numpy.where((by_left <= by_above) & (by_left <= by_corner), left, numpy.where(by_above <= by_corner, above, corner))
    kinds = [plain, plain - left, plain - above, plain - (left + above) // 2, plain - nearest]
    cost = numpy.stack([numpy.abs(((kind + 128) & 255) - 128).sum(axis=1) for kind in kinds])
    chosen = cost.argmin(axis=0)
    rows = numpy.zeros((height, width * 4 + 1), dtype=numpy.uint8)
    rows[:, 0] = chosen
    for number, kind in enumerate(kinds):
        rows[chosen == number, 1:] = (kind[chosen == number] & 255).astype(numpy.uint8)

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return SIGNATURE + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows.tobytes(), 9)) + chunk(b"IEND", b"")


def pieces(solid):
    height, width = solid.shape
    seen = numpy.zeros_like(solid)
    found = []
    for start_y, start_x in zip(*numpy.nonzero(solid[::7, ::7])):
        y0, x0 = int(start_y) * 7, int(start_x) * 7
        if seen[y0, x0]:
            continue
        queue = collections.deque([(y0, x0)])
        seen[y0, x0] = True
        members = []
        while queue:
            y, x = queue.popleft()
            members.append((y, x))
            for ny, nx in ((y - 1, x), (y + 1, x), (y, x - 1), (y, x + 1)):
                if 0 <= ny < height and 0 <= nx < width and solid[ny, nx] and not seen[ny, nx]:
                    seen[ny, nx] = True
                    queue.append((ny, nx))
        found.append(numpy.array(members))
    return found


def the_tile(pixels):
    best = None
    for members in pieces(pixels[:, :, 3] > SOLID):
        top, left = members.min(axis=0)
        bottom, right = members.max(axis=0)
        tall, wide = bottom - top + 1, right - left + 1
        if min(tall, wide) < 64:
            continue
        square = min(tall, wide) / max(tall, wide)
        filled = len(members) / (tall * wide)
        if square > 0.9 and filled > 0.85 and (best is None or len(members) > len(best[0])):
            best = (members, int(top), int(left), int(tall), int(wide), square, filled)
    if best is None:
        fail("no filled, square shape was found in the picture")
    members, top, left, tall, wide, square, filled = best
    side = max(tall, wide)
    tile = numpy.zeros((side, side, 4), dtype=numpy.uint8)
    off_y, off_x = (side - tall) // 2, (side - wide) // 2
    ys, xs = members[:, 0], members[:, 1]
    tile[ys - top + off_y, xs - left + off_x] = pixels[ys, xs]
    alpha = tile[:, :, 3].astype(numpy.float64)
    tile[:, :, 3] = numpy.clip(numpy.round(alpha * 255.0 / FULLY_SOLID_FROM), 0, 255).astype(numpy.uint8)
    print(f"the tile: {wide} x {tall} pixels at ({left}, {top}), {filled:.1%} of its box filled")
    return tile


def to_linear(values):
    values = values / 255.0
    return numpy.where(values <= 0.04045, values / 12.92, ((values + 0.055) / 1.055) ** 2.4)


def from_linear(values):
    values = numpy.clip(values, 0.0, 1.0)
    return numpy.where(values <= 0.0031308, values * 12.92, 1.055 * values ** (1 / 2.4) - 0.055)


def shares(source, target):
    weights = numpy.zeros((target, source))
    step = source / target
    for index in range(target):
        start, end = index * step, (index + 1) * step
        first, last = int(numpy.floor(start)), min(int(numpy.ceil(end)), source)
        for cell in range(first, last):
            weights[index, cell] = min(end, cell + 1) - max(start, cell)
    return weights / step


def shrink(pixels, size):
    side = pixels.shape[0]
    if size > side:
        fail(f"the tile is only {side} pixels wide, too small for a {size} pixel icon")
    alpha = pixels[:, :, 3].astype(numpy.float64) / 255.0
    light = to_linear(pixels[:, :, :3].astype(numpy.float64)) * alpha[:, :, None]
    weights = shares(side, size)
    small_alpha = weights @ alpha @ weights.T
    small_light = numpy.stack([weights @ light[:, :, band] @ weights.T for band in range(3)], axis=2)
    safe = numpy.where(small_alpha > 1e-6, small_alpha, 1.0)
    colour = from_linear(small_light / safe[:, :, None])
    out = numpy.zeros((size, size, 4), dtype=numpy.uint8)
    out[:, :, :3] = numpy.round(colour * 255.0).astype(numpy.uint8)
    out[:, :, 3] = numpy.round(numpy.clip(small_alpha, 0.0, 1.0) * 255.0).astype(numpy.uint8)
    out[out[:, :, 3] == 0] = 0
    return out


def bitmap(pixels):
    size = pixels.shape[0]
    header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, size * size * 4, 0, 0, 0, 0)
    colour = pixels[::-1, :, [2, 1, 0, 3]].tobytes()
    row = ((size + 31) // 32) * 4
    mask = numpy.zeros((size, row), dtype=numpy.uint8)
    clear = pixels[::-1, :, 3] == 0
    for x in range(size):
        mask[:, x // 8] |= (clear[:, x].astype(numpy.uint8) << (7 - x % 8))
    return header + colour + mask.tobytes()


def icon_file(images):
    entries = [(size, write_png(pixels) if size >= PNG_FROM else bitmap(pixels)) for size, pixels in images]
    out = struct.pack("<HHH", 0, 1, len(entries))
    at = 6 + 16 * len(entries)
    for size, body in entries:
        out += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(body), at)
        at += len(body)
    return out + b"".join(body for _, body in entries)


def main():
    parser = argparse.ArgumentParser(
        description="Cut the square app tile out of a picture and write the program's icon files from it."
    )
    parser.add_argument("picture", help="a PNG, or an SVG that carries one")
    parser.add_argument("--into", default=str(ROOT / "ps-app" / "ui"), help="where app-icon.ico and app-icon.png go")
    args = parser.parse_args()
    tile = the_tile(read_png(picture_bytes(pathlib.Path(args.picture))))
    images = [(size, shrink(tile, size)) for size in SIZES]
    into = pathlib.Path(args.into)
    into.mkdir(parents=True, exist_ok=True)
    (into / "app-icon.ico").write_bytes(icon_file(images))
    (into / "app-icon.png").write_bytes(write_png(images[0][1]))
    for name in ("app-icon.ico", "app-icon.png"):
        print(f"{name}: {(into / name).stat().st_size} bytes")
    print("sizes in the icon:", ", ".join(str(size) for size, _ in images))


if __name__ == "__main__":
    main()
