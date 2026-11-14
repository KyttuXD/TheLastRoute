#!/usr/bin/env python3
"""Writes an .icns that OS X 10.5 and 10.6 can read, from the game's own logo PNGs. Standard library only.

Why not `iconutil`: it writes PNG icon entries, which OS X 10.5 and 10.6 do not read; a bundle whose icns they cannot
read shows the generic application icon. These systems read the classic entries: is32/s8mk (16 px), il32/l8mk (32 px)
and it32/t8mk (128 px), RLE-packed 24-bit colour plus an 8-bit mask. A PNG entry for 128 px (ic07) and 256 px (ic08)
is added for newer systems; older ones skip entry types they do not know.

    python3 make-icns.py OUT.icns BRANDING_DIR        write the icon from BRANDING_DIR/logo_{16,32,128,256}.png
    python3 make-icns.py --verify FILE.icns           parse a file and check every classic entry decodes
"""
import struct
import sys
import zlib


def read_png(path):
    """Returns (width, height, rgba bytes) of an 8-bit, non-interlaced RGB or RGBA PNG."""
    data = open(path, "rb").read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit(f"{path}: not a PNG")
    pos, idat, width = 8, [], None
    while pos < len(data):
        n, kind = struct.unpack_from(">I4s", data, pos)
        body = data[pos + 8 : pos + 8 + n]
        pos += 12 + n
        if kind == b"IHDR":
            width, height, depth, ctype, _, _, inter = struct.unpack(">IIBBBBB", body)
            if depth != 8 or ctype not in (2, 6) or inter != 0:
                raise SystemExit(f"{path}: need 8-bit RGB or RGBA, not interlaced")
        elif kind == b"IDAT":
            idat.append(body)
        elif kind == b"IEND":
            break
    bpp = 4 if ctype == 6 else 3
    raw = zlib.decompress(b"".join(idat))
    stride = width * bpp
    rows, prev = [], bytearray(stride)
    for y in range(height):
        base = y * (stride + 1)
        filt, line = raw[base], bytearray(raw[base + 1 : base + 1 + stride])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            if filt == 1:
                line[i] = (line[i] + a) & 255
            elif filt == 2:
                line[i] = (line[i] + b) & 255
            elif filt == 3:
                line[i] = (line[i] + ((a + b) >> 1)) & 255
            elif filt == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[i] = (line[i] + pred) & 255
            elif filt != 0:
                raise SystemExit(f"{path}: bad PNG filter {filt}")
        rows.append(line)
        prev = line
    out = bytearray()
    for line in rows:
        if bpp == 4:
            out += line
        else:
            for x in range(width):
                out += line[x * 3 : x * 3 + 3] + b"\xff"
    return width, height, bytes(out)


def rle(channel):
    """The icns run-length packing of one colour channel: 0..127 = copy n+1 bytes, 128..255 = repeat the next byte n-125 times."""
    out, i, n = bytearray(), 0, len(channel)
    while i < n:
        run = 1
        while i + run < n and channel[i + run] == channel[i] and run < 130:
            run += 1
        if run >= 3:
            out += bytes([run - 3 + 128, channel[i]])
            i += run
            continue
        start = i
        i += run
        while i < n and i - start < 128:
            if i + 2 < n and channel[i] == channel[i + 1] == channel[i + 2]:
                break
            i += 1
        out += bytes([i - start - 1]) + bytes(channel[start:i])
    return bytes(out)


def unrle(data, count):
    out, i = bytearray(), 0
    while len(out) < count:
        h = data[i]
        i += 1
        if h < 128:
            out += data[i : i + h + 1]
            i += h + 1
        else:
            out += bytes([data[i]]) * (h - 125)
            i += 1
    if len(out) != count:
        raise ValueError("RLE channel has the wrong length")
    return bytes(out)


def planes(rgba):
    return [rgba[c::4] for c in range(4)]


def entry(tag, payload):
    return tag + struct.pack(">I", 8 + len(payload)) + payload


def colour_entries(color_tag, mask_tag, rgba, header=b""):
    r, g, b, a = planes(rgba)
    return entry(color_tag, header + rle(r) + rle(g) + rle(b)) + entry(mask_tag, a)


def build(branding):
    parts = []
    sizes = {}
    for px in (16, 32, 128, 256):
        w, h, rgba = read_png(f"{branding}/logo_{px}.png")
        if (w, h) != (px, px):
            raise SystemExit(f"logo_{px}.png is {w}x{h}")
        sizes[px] = rgba
    parts.append(colour_entries(b"is32", b"s8mk", sizes[16]))
    parts.append(colour_entries(b"il32", b"l8mk", sizes[32]))
    parts.append(colour_entries(b"it32", b"t8mk", sizes[128], header=b"\0\0\0\0"))
    for px, tag in ((128, b"ic07"), (256, b"ic08")):
        parts.append(entry(tag, open(f"{branding}/logo_{px}.png", "rb").read()))
    body = b"".join(parts)
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def verify(path):
    data = open(path, "rb").read()
    if data[:4] != b"icns" or struct.unpack_from(">I", data, 4)[0] != len(data):
        raise SystemExit("not an icns, or the length is wrong")
    pos, found = 8, {}
    while pos < len(data):
        tag, n = struct.unpack_from(">4sI", data, pos)
        found[tag] = data[pos + 8 : pos + n]
        pos += n
    if pos != len(data):
        raise SystemExit("entries do not fill the file")
    for color, mask, px, header in ((b"is32", b"s8mk", 16, 0), (b"il32", b"l8mk", 32, 0), (b"it32", b"t8mk", 128, 4)):
        if color not in found or mask not in found:
            raise SystemExit(f"missing {color.decode()}/{mask.decode()}")
        if len(found[mask]) != px * px:
            raise SystemExit(f"{mask.decode()} has the wrong size")
        body, count = found[color][header:], px * px
        for _ in range(3):  # three channels, one after the other
            # walk one channel to find where it ends
            got, i = 0, 0
            while got < count:
                h = body[i]
                i += 1
                if h < 128:
                    got += h + 1
                    i += h + 1
                else:
                    got += h - 125
                    i += 1
            if got != count:
                raise SystemExit(f"{color.decode()}: a channel is not {count} bytes")
            unrle(body[:i], count)
            body = body[i:]
        if body:
            raise SystemExit(f"{color.decode()}: bytes left over")
    print("icns ok:", ", ".join(sorted(t.decode() for t in found)))


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--verify":
        verify(sys.argv[2])
    elif len(sys.argv) == 3:
        open(sys.argv[1], "wb").write(build(sys.argv[2]))
        verify(sys.argv[1])
    else:
        raise SystemExit(__doc__)
