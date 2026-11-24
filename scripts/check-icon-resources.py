"""Check the executable's icon group and every referenced image against the asset."""
from pathlib import Path
import struct
import sys

ASSET = Path(__file__).resolve().parents[1] / 'assets' / 'nest-desktop.ico'


def icon_payloads():
    icon = ASSET.read_bytes()
    assert struct.unpack_from('<HH', icon) == (0, 1)
    result = {}
    for index in range(struct.unpack_from('<H', icon, 4)[0]):
        width, height, _, _, planes, depth, length, offset = struct.unpack_from('<BBBBHHII', icon, 6 + index * 16)
        assert planes == 1 and depth == 32
        result[(width or 256, height or 256)] = icon[offset:offset + length]
    return result


def check(path):
    data = path.read_bytes()
    pe = struct.unpack_from('<I', data, 60)[0]
    coff = pe + 4
    count = struct.unpack_from('<H', data, coff + 2)[0]
    optional_size = struct.unpack_from('<H', data, coff + 16)[0]
    optional = coff + 20
    directories = optional + (112 if struct.unpack_from('<H', data, optional)[0] == 0x20B else 96)
    resource_rva, resource_size = struct.unpack_from('<II', data, directories + 16)
    assert resource_rva and resource_size
    sections = []
    for index in range(count):
        header = optional + optional_size + index * 40
        virtual_size, virtual, raw_size, raw = struct.unpack_from('<IIII', data, header + 8)
        sections.append((virtual, max(virtual_size, raw_size), raw))

    def file_offset(rva):
        for virtual, size, raw in sections:
            if virtual <= rva < virtual + size:
                return raw + rva - virtual
        raise AssertionError('Unmapped resource RVA')

    root = file_offset(resource_rva)

    def entries(relative):
        named, numbered = struct.unpack_from('<HH', data, root + relative + 12)
        return [struct.unpack_from('<II', data, root + relative + 16 + 8 * index)
                for index in range(named + numbered)]

    def child(relative, identifier):
        entry = next(value for key, value in entries(relative) if key == identifier)
        assert entry & 0x80000000
        return entry & 0x7FFFFFFF

    def resource(kind, identifier):
        language = entries(child(child(0, kind), identifier))[0][1]
        assert not language & 0x80000000
        rva, size, _, _ = struct.unpack_from('<IIII', data, root + language)
        offset = file_offset(rva)
        payload = data[offset:offset + size]
        assert len(payload) == size
        return payload

    icon_groups = entries(child(0, 14))
    assert len(icon_groups) == 1
    group = resource(14, icon_groups[0][0])
    assert struct.unpack_from('<HH', group) == (0, 1)
    count = struct.unpack_from('<H', group, 4)[0]
    expected = icon_payloads()
    assert count == len(expected)
    observed = set()
    for index in range(count):
        width, height, _, _, planes, depth, length, identifier = struct.unpack_from('<BBBBHHIH', group, 6 + index * 14)
        size = width or 256, height or 256
        assert size not in observed
        observed.add(size)
        assert planes == 1 and depth == 32
        payload = resource(3, identifier)
        assert len(payload) == length
        assert payload == expected[size], f'Icon image mismatch at {size}'
    assert observed == set(expected)
    print(f'{path.name}: {count} matching RT_ICON images and RT_GROUP_ICON verified ({sorted(w for w, _ in observed)} px)')


if __name__ == '__main__':
    check(Path(sys.argv[1]).resolve())
