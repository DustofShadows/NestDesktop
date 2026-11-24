"""Read PE import tables without loading the executable or external packages.

This checks direct and delay-imported DLL names, not every possible dynamic
LoadLibrary call. The allowlist covers the Windows APIs used by NestDesktop.
"""

import argparse
import json
import struct
from pathlib import Path


WINDOWS_DLLS = {
    "advapi32.dll", "bcrypt.dll", "bcryptprimitives.dll", "comctl32.dll",
    "comdlg32.dll", "crypt32.dll", "d2d1.dll", "d3d11.dll", "dcomp.dll",
    "dwmapi.dll", "dwrite.dll", "dxgi.dll", "gdi32.dll", "imm32.dll",
    "kernel32.dll", "kernelbase.dll", "msvcrt.dll", "ntdll.dll", "ole32.dll",
    "oleaut32.dll", "opengl32.dll", "powrprof.dll", "propsys.dll", "rpcrt4.dll",
    "secur32.dll", "setupapi.dll", "shell32.dll", "shcore.dll", "shlwapi.dll",
    "ucrtbase.dll", "user32.dll", "userenv.dll", "uxtheme.dll", "version.dll",
    "windowscodecs.dll", "winmm.dll", "winspool.drv", "wintrust.dll", "ws2_32.dll",
}


def imports(path: Path):
    data = path.read_bytes()
    if data[:2] != b"MZ":
        raise ValueError("Not a Windows executable")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("Invalid PE signature")
    machine, section_count = struct.unpack_from("<HH", data, pe + 4)
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    optional = pe + 24
    magic = struct.unpack_from("<H", data, optional)[0]
    if magic == 0x20B:
        directories = optional + 112
        image_base = struct.unpack_from("<Q", data, optional + 24)[0]
    elif magic == 0x10B:
        directories = optional + 96
        image_base = struct.unpack_from("<I", data, optional + 28)[0]
    else:
        raise ValueError("Unsupported PE optional-header format")
    sections = []
    for index in range(section_count):
        entry = optional + optional_size + index * 40
        virtual_size, virtual_address, raw_size, raw_offset = struct.unpack_from("<IIII", data, entry + 8)
        sections.append((virtual_address, max(virtual_size, raw_size), raw_offset))

    def offset(rva):
        for address, length, raw in sections:
            if address <= rva < address + length:
                return raw + rva - address
        if rva < optional + optional_size:
            return rva
        raise ValueError("Import RVA is outside the mapped PE sections")

    def name_at(rva):
        start = offset(rva)
        return data[start:data.index(b"\0", start)].decode("ascii")

    names = set()
    import_rva, import_size = struct.unpack_from("<II", data, directories + 8)
    if import_rva and import_size:
        cursor = offset(import_rva)
        while True:
            descriptor = struct.unpack_from("<IIIII", data, cursor)
            if not any(descriptor):
                break
            names.add(name_at(descriptor[3]))
            cursor += 20
    delay_rva, delay_size = struct.unpack_from("<II", data, directories + 13 * 8)
    if delay_rva and delay_size:
        cursor = offset(delay_rva)
        while True:
            descriptor = struct.unpack_from("<IIIIIIII", data, cursor)
            if not any(descriptor):
                break
            name_rva = descriptor[1] if descriptor[0] & 1 else descriptor[1] - image_base
            names.add(name_at(name_rva))
            cursor += 32
    unknown = sorted(name for name in names if name.lower() not in WINDOWS_DLLS
                     and not name.lower().startswith(("api-ms-win-", "ext-ms-win-")))
    return {
        "executable": str(path.resolve()),
        "machine": hex(machine),
        "imports": sorted(names, key=str.lower),
        "non_allowlisted_imports": unknown,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    args = parser.parse_args()
    result = imports(args.executable)
    print(json.dumps(result, indent=2))
    raise SystemExit(1 if result["non_allowlisted_imports"] else 0)

