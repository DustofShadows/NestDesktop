"""Rebuild the Windows icon from SetupWindow.DrawMark's native geometry.

No image libraries are required. Each size is rasterized independently with
8x8 coverage samples, so the tray size is not a scaled-down large bitmap.
"""
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parents[1]
SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)
COLORS = ((201, 188, 255), (108, 90, 210), (143, 125, 231),
          (97, 159, 178), (233, 173, 117))
SHAPES = ((1, 1, 55, 55, 14), (12, 12, 14, 14, 4),
          (30, 12, 14, 14, 4), (12, 30, 14, 14, 4),
          (30, 30, 14, 14, 4))


def inside(x, y, shape):
    left, top, width, height, radius = shape
    if not (left <= x < left + width and top <= y < top + height):
        return False
    nearest_x = min(max(x, left + radius), left + width - radius)
    nearest_y = min(max(y, top + radius), top + height - radius)
    return (x - nearest_x) ** 2 + (y - nearest_y) ** 2 <= radius ** 2


def raster(size):
    rgba = bytearray()
    samples = 8
    for py in range(size):
        for px in range(size):
            counts = [0] * len(SHAPES)
            for sy in range(samples):
                y = (py + (sy + 0.5) / samples) * 58 / size
                for sx in range(samples):
                    x = (px + (sx + 0.5) / samples) * 58 / size
                    found = None
                    for index, shape in enumerate(SHAPES):
                        if inside(x, y, shape):
                            found = index
                    if found is not None:
                        counts[found] += 1
            coverage = sum(counts)
            if coverage:
                rgba.extend(round(sum(count * color[channel] for count, color in zip(counts, COLORS)) / coverage)
                            for channel in range(3))
                rgba.append(round(coverage * 255 / (samples * samples)))
            else:
                rgba.extend((0, 0, 0, 0))
    return bytes(rgba)


def png(size, rgba):
    def chunk(kind, payload):
        return struct.pack('>I', len(payload)) + kind + payload + struct.pack('>I', zlib.crc32(kind + payload))
    raw = b''.join(b'\0' + rgba[y * size * 4:(y + 1) * size * 4] for y in range(size))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))


def verify_pixels(size, rgba):
    pixel = lambda x, y: tuple(rgba[(y * size + x) * 4:(y * size + x + 1) * 4])
    assert pixel(0, 0)[3] == 0
    assert any(0 < rgba[index] < 255 for index in range(3, len(rgba), 4))
    for (x, y), expected in zip(((19, 19), (37, 19), (19, 37), (37, 37)), COLORS[1:]):
        assert pixel(int(x * size / 58), int(y * size / 58)) == expected + (255,)


def main():
    entries = []
    payloads = []
    offset = 6 + 16 * len(SIZES)
    for size in SIZES:
        rgba = raster(size)
        verify_pixels(size, rgba)
        data = png(size, rgba)
        entries.append(struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
        payloads.append(data)
        offset += len(data)
    destination = ROOT / 'assets' / 'nest-desktop.ico'
    destination.write_bytes(struct.pack('<HHH', 0, 1, len(entries)) + b''.join(entries) + b''.join(payloads))
    print(f'{destination.name}: {len(entries)} independently antialiased sizes, exact palette and transparent corners verified')


if __name__ == '__main__':
    main()
