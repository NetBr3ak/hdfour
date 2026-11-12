"""Seed each target from the committed C-library fixtures, without extra packages."""

import hashlib
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def save(target: str, content: bytes) -> None:
    directory = ROOT / "fuzz" / "corpus" / target
    directory.mkdir(parents=True, exist_ok=True)
    (directory / hashlib.sha256(content).hexdigest()).write_bytes(content)


def descriptors(content: bytes) -> dict[tuple[int, int], bytes]:
    elements = {}
    offset = 4
    visited = set()
    while offset:
        if offset in visited:
            raise ValueError("fixture has a DD cycle")
        visited.add(offset)
        count, following = struct.unpack_from(">HI", content, offset)
        for index in range(count):
            tag, reference, start, length = struct.unpack_from(
                ">HHII", content, offset + 6 + 12 * index
            )
            if tag != 1 and start != 0xFFFFFFFF:
                elements[tag, reference] = content[start : start + length]
        offset = following
    return elements


def main() -> None:
    for path in sorted((ROOT / "fixtures").glob("F??.hdf")):
        content = path.read_bytes()
        save("open_read_all", content)
        elements = descriptors(content)
        for (tag, _), header in elements.items():
            if tag & 0xC000 != 0x4000 or len(header) < 14:
                continue
            # hcomp.c:608-615 stores special, version, length, ref, model, codec.
            special, version, length, reference, model, codec = struct.unpack_from(
                ">HHIHHH", header
            )
            if special != 3 or version != 0 or model != 0 or not 0 < length <= 65535:
                continue
            if codec not in (1, 4):
                continue
            payload = elements[40, reference]
            target = "codec_rle" if codec == 1 else "codec_deflate"
            save(target, struct.pack("<H", length) + payload)
    for path in sorted((ROOT / "fixtures" / "odl").glob("*.odl")):
        save("odl", path.read_bytes())
    save("codec_rle", b"\x03\x00\x80\x2a")
    save("codec_deflate", b"\x00\x00\x78\x9c\x03\x00\x00\x00\x00\x01")
    for target in ("open_read_all", "codec_deflate", "codec_rle", "odl"):
        print(f"seeded {target}")


if __name__ == "__main__":
    main()
