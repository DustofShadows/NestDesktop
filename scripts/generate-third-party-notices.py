#!/usr/bin/env python3
"""Generate the release notices from cached locked crates and the Rust sysroot.

Requires Python 3.8+ and an already-fetched Cargo registry; never downloads files.
The complete lockfile is covered, including build-only and other-target crates.
This intentionally inclusive inventory does not claim every crate is in the EXE.
"""

import argparse
import hashlib
import html
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import re
import subprocess
import sys


MINGW_VERSION = "12.0.0"
MINGW_RUNTIME_SHA256 = "e9b2dc02451ea29092a1f25fa0f3c07207ed421f1807dffb0c4e6dce69dee7bd"
WINPTHREADS_SHA256 = "63263614cdd29f2f93cba85e992f041b31f9fc7b4033692f31269489a8a1b177"
MINGW_RUNTIME = "MinGW-w64 runtime licensing\n***************************\n\nThis program or library was built using MinGW-w64 and statically\nlinked against the MinGW-w64 runtime. Some parts of the runtime\nare under licenses which require that the copyright and license\nnotices are included when distributing the code in binary form.\nThese notices are listed below.\n\n\n========================\nOverall copyright notice\n========================\n\nCopyright (c) 2009, 2010, 2011, 2012, 2013 by the mingw-w64 project\n\nThis license has been certified as open source. It has also been designated\nas GPL compatible by the Free Software Foundation (FSF).\n\nRedistribution and use in source and binary forms, with or without\nmodification, are permitted provided that the following conditions are met:\n\n   1. Redistributions in source code must retain the accompanying copyright\n      notice, this list of conditions, and the following disclaimer.\n   2. Redistributions in binary form must reproduce the accompanying\n      copyright notice, this list of conditions, and the following disclaimer\n      in the documentation and/or other materials provided with the\n      distribution.\n   3. Names of the copyright holders must not be used to endorse or promote\n      products derived from this software without prior written permission\n      from the copyright holders.\n   4. The right to distribute this software or to use it for any purpose does\n      not give you the right to use Servicemarks (sm) or Trademarks (tm) of\n      the copyright holders.  Use of them is covered by separate agreement\n      with the copyright holders.\n   5. If any files are modified, you must cause the modified files to carry\n      prominent notices stating that you changed the files and the date of\n      any change.\n\nDisclaimer\n\nTHIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS ``AS IS'' AND ANY EXPRESSED\nOR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES\nOF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO\nEVENT SHALL THE COPYRIGHT HOLDERS BE LIABLE FOR ANY DIRECT, INDIRECT,\nINCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT\nLIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA,\nOR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF\nLIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING\nNEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE,\nEVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.\n\n======================================== \ngetopt, getopt_long, and getop_long_only\n======================================== \n\nCopyright (c) 2002 Todd C. Miller <Todd.Miller@courtesan.com> \n \nPermission to use, copy, modify, and distribute this software for any \npurpose with or without fee is hereby granted, provided that the above \ncopyright notice and this permission notice appear in all copies. \n \t \nTHE SOFTWARE IS PROVIDED \"AS IS\" AND THE AUTHOR DISCLAIMS ALL WARRANTIES\nWITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF\nMERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR\nANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES\nWHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN\nACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF\nOR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.\n\nSponsored in part by the Defense Advanced Research Projects\nAgency (DARPA) and Air Force Research Laboratory, Air Force\nMateriel Command, USAF, under agreement number F39502-99-1-0512.\n\n        *       *       *       *       *       *       * \n\nCopyright (c) 2000 The NetBSD Foundation, Inc.\nAll rights reserved.\n\nThis code is derived from software contributed to The NetBSD Foundation\nby Dieter Baron and Thomas Klausner.\n\nRedistribution and use in source and binary forms, with or without\nmodification, are permitted provided that the following conditions\nare met:\n 1. Redistributions of source code must retain the above copyright\n    notice, this list of conditions and the following disclaimer.\n 2. Redistributions in binary form must reproduce the above copyright\n    notice, this list of conditions and the following disclaimer in the\n    documentation and/or other materials provided with the distribution.\n\nTHIS SOFTWARE IS PROVIDED BY THE NETBSD FOUNDATION, INC. AND CONTRIBUTORS\n``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED\nTO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR\nPURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE FOUNDATION OR CONTRIBUTORS\nBE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR\nCONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF\nSUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS\nINTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN\nCONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)\nARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE\nPOSSIBILITY OF SUCH DAMAGE.\n\n\n===============================================================\ngdtoa: Converting between IEEE floating point numbers and ASCII\n===============================================================\n\nThe author of this software is David M. Gay.\n\nCopyright (C) 1997, 1998, 1999, 2000, 2001 by Lucent Technologies\nAll Rights Reserved\n\nPermission to use, copy, modify, and distribute this software and\nits documentation for any purpose and without fee is hereby\ngranted, provided that the above copyright notice appear in all\ncopies and that both that the copyright notice and this\npermission notice and warranty disclaimer appear in supporting\ndocumentation, and that the name of Lucent or any of its entities\nnot be used in advertising or publicity pertaining to\ndistribution of the software without specific, written prior\npermission.\n\nLUCENT DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE,\nINCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS.\nIN NO EVENT SHALL LUCENT OR ANY OF ITS ENTITIES BE LIABLE FOR ANY\nSPECIAL, INDIRECT OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES\nWHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER\nIN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,\nARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF\nTHIS SOFTWARE.\n\n        *       *       *       *       *       *       *\n\nThe author of this software is David M. Gay.\n\nCopyright (C) 2005 by David M. Gay\nAll Rights Reserved\n\nPermission to use, copy, modify, and distribute this software and its\ndocumentation for any purpose and without fee is hereby granted,\nprovided that the above copyright notice appear in all copies and that\nboth that the copyright notice and this permission notice and warranty\ndisclaimer appear in supporting documentation, and that the name of\nthe author or any of his current or former employers not be used in\nadvertising or publicity pertaining to distribution of the software\nwithout specific, written prior permission.\n\nTHE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE,\nINCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS.  IN\nNO EVENT SHALL THE AUTHOR OR ANY OF HIS CURRENT OR FORMER EMPLOYERS BE\nLIABLE FOR ANY SPECIAL, INDIRECT OR CONSEQUENTIAL DAMAGES OR ANY\nDAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,\nWHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,\nARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS\nSOFTWARE.\n\n        *       *       *       *       *       *       *\n\nThe author of this software is David M. Gay.\n\nCopyright (C) 2004 by David M. Gay.\nAll Rights Reserved\nBased on material in the rest of /netlib/fp/gdota.tar.gz,\nwhich is copyright (C) 1998, 2000 by Lucent Technologies.\n\nPermission to use, copy, modify, and distribute this software and\nits documentation for any purpose and without fee is hereby\ngranted, provided that the above copyright notice appear in all\ncopies and that both that the copyright notice and this\npermission notice and warranty disclaimer appear in supporting\ndocumentation, and that the name of Lucent or any of its entities\nnot be used in advertising or publicity pertaining to\ndistribution of the software without specific, written prior\npermission.\n\nLUCENT DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE,\nINCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS.\nIN NO EVENT SHALL LUCENT OR ANY OF ITS ENTITIES BE LIABLE FOR ANY\nSPECIAL, INDIRECT OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES\nWHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER\nIN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,\nARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF\nTHIS SOFTWARE.\n\n\n=========================\nParts of the math library\n=========================\n\nCopyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.\n\nDeveloped at SunSoft, a Sun Microsystems, Inc. business.\nPermission to use, copy, modify, and distribute this\nsoftware is freely granted, provided that this notice\nis preserved.\n\n        *       *       *       *       *       *       *\n\nCopyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.\n\nDeveloped at SunPro, a Sun Microsystems, Inc. business.\nPermission to use, copy, modify, and distribute this\nsoftware is freely granted, provided that this notice\nis preserved.\n\n        *       *       *       *       *       *       *\n\nFIXME: Cephes math lib\nCopyright (C) 1984-1998 Stephen L. Moshier\n\nIt sounds vague, but as to be found at\n<http://lists.debian.org/debian-legal/2004/12/msg00295.html>, it gives an\nimpression that the author could be willing to give an explicit\npermission to distribute those files e.g. under a BSD style license. So\nprobably there is no problem here, although it could be good to get a\npermission from the author and then add a license into the Cephes files\nin MinGW runtime. At least on follow-up it is marked that debian sees the\nversion a-like BSD one. As MinGW.org (where those cephes parts are coming\nfrom) distributes them now over 6 years, it should be fine.\n\n===================================\nHeaders and IDLs imported from Wine\n===================================\n\nSome header and IDL files were imported from the Wine project. These files\nare prominent maked in source. Their copyright belongs to contributors and\nthey are distributed under LGPL license.\n\nDisclaimer\n\nThis library is free software; you can redistribute it and/or\nmodify it under the terms of the GNU Lesser General Public\nLicense as published by the Free Software Foundation; either\nversion 2.1 of the License, or (at your option) any later version.\n\nThis library is distributed in the hope that it will be useful,\nbut WITHOUT ANY WARRANTY; without even the implied warranty of\nMERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU\nLesser General Public License for more details.\n"
WINPTHREADS = "Copyright (c) 2011 mingw-w64 project\n\nPermission is hereby granted, free of charge, to any person obtaining a\ncopy of this software and associated documentation files (the \"Software\"),\nto deal in the Software without restriction, including without limitation\nthe rights to use, copy, modify, merge, publish, distribute, sublicense,\nand/or sell copies of the Software, and to permit persons to whom the\nSoftware is furnished to do so, subject to the following conditions:\n\nThe above copyright notice and this permission notice shall be included in\nall copies or substantial portions of the Software.\n\nTHE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR\nIMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,\nFITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE\nAUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\nLIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING\nFROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER\nDEALINGS IN THE SOFTWARE.\n\n\n/*\n * Parts of this library are derived by:\n *\n * Posix Threads library for Microsoft Windows\n *\n * Use at own risk, there is no implied warranty to this code.\n * It uses undocumented features of Microsoft Windows that can change\n * at any time in the future.\n *\n * (C) 2010 Lockless Inc.\n * All rights reserved.\n *\n * Redistribution and use in source and binary forms, with or without modification,\n * are permitted provided that the following conditions are met:\n *\n *\n *  * Redistributions of source code must retain the above copyright notice,\n *    this list of conditions and the following disclaimer.\n *  * Redistributions in binary form must reproduce the above copyright notice,\n *    this list of conditions and the following disclaimer in the documentation\n *    and/or other materials provided with the distribution.\n *  * Neither the name of Lockless Inc. nor the names of its contributors may be\n *    used to endorse or promote products derived from this software without\n *    specific prior written permission.\n *\n * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS \"AS IS\" AN\n * ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED\n * WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED.\n * IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT,\n * INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING,\n * BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,\n * DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF\n * LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE\n * OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED\n * OF THE POSSIBILITY OF SUCH DAMAGE.\n */\n"


def read(path):
    return Path(path).read_text(encoding="utf-8-sig").replace("\r\n", "\n")


def string_field(block, name):
    # Cargo's normalized cached manifests and lockfile use quoted scalar fields.
    # Avoid requiring a non-standard TOML parser on Python 3.8 through 3.10.
    match = re.search(r"(?m)^" + re.escape(name) + r'\s*=\s*("[^\n]*")\s*$', block)
    return json.loads(match.group(1)) if match else None


class PlainReport(HTMLParser):
    """Retain all attribution text, with explicit references to deduplicated texts."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.parts = []

    def handle_starttag(self, tag, attrs):
        if tag in ("p", "div", "h1", "h2", "h3", "li", "summary"):
            self.parts.append("\n")

    def handle_endtag(self, tag):
        if tag in ("p", "div", "h1", "h2", "h3", "li", "summary"):
            self.parts.append("\n")

    def handle_data(self, data):
        self.parts.append(data)

    def text(self):
        lines = [re.sub(r"\s+", " ", line).strip() for line in "".join(self.parts).splitlines()]
        return re.sub(r"\n{3,}", "\n\n", "\n".join(lines)).strip()


class Notices:
    def __init__(self):
        self.items = {}

    def add(self, text):
        text = text.replace("\r\n", "\n").strip("\n") + "\n"
        digest = hashlib.sha256(text.encode("utf-8")).hexdigest()
        self.items.setdefault(digest, text)
        return "notice-" + digest[:16]

    def link(self, text):
        key = self.add(text)
        return "[Full text](#" + key + ")"


def find_crate(cargo_home, name, version):
    matches = sorted((cargo_home / "registry" / "src").glob("*/" + name + "-" + version))
    if not matches:
        raise ValueError("Missing cached crate: " + name + " " + version + "; run cargo fetch --locked first")
    # Cargo registry archives identify exact name/version pairs; choose a stable cache.
    return matches[0]


def run(*args):
    return subprocess.check_output(args, text=True, encoding="utf-8", stderr=subprocess.STDOUT).strip()


def generate(args):
    notices = Notices()
    lock = read(args.lockfile)
    crates = []
    for block in lock.split("[[package]]")[1:]:
        if string_field(block, "source"):
            name, version = string_field(block, "name"), string_field(block, "version")
            if not name or not version:
                raise ValueError("Unsupported lockfile package metadata")
            crates.append((name, version))
    sections = [
        "# Third-party notices\n",
        "NestDesktop's own code is covered by [LICENSE](LICENSE). The components below retain their upstream licences and copyrights.\n",
        "This is an intentionally inclusive notice inventory for the locked dependencies and the build toolchain. It includes build-time crates, other Windows targets, and standard-library components that may not be present in this particular executable. Their inclusion does not change NestDesktop's licence.\n",
        "Generated offline by `scripts/generate-third-party-notices.py`. Regenerate when Cargo.lock or the release toolchain changes. Exact repeated notice texts are stored once and referenced below.\n",
        "## Cargo dependencies\n",
        "| Crate | Declared licence | Included notices |\n|---|---|---|",
    ]
    for name, version in sorted(crates):
        directory = find_crate(args.cargo_home, name, version)
        manifest = read(directory / "Cargo.toml")
        package = manifest.split("[package]", 1)[1].split("\n[", 1)[0]
        licence = string_field(package, "license")
        if not licence:
            raise ValueError("Missing declared licence for " + name)
        files = sorted((p for p in directory.iterdir() if p.is_file() and p.name.casefold().startswith(("license", "copying", "notice", "unlicense"))), key=lambda p: p.name.casefold())
        if not files:
            raise ValueError("Missing licence text for " + name)
        links = ["`" + p.name + "`: " + notices.link(read(p)) for p in files]
        url = "https://crates.io/crates/" + name + "/" + version
        sections.append("| [" + name + " " + version + "](" + url + ") | " + licence + " | " + "; ".join(links) + " |")

    report_path = args.sysroot / "share" / "doc" / "rust" / "COPYRIGHT-library.html"
    report = read(report_path)
    version = run(args.rustc, "--version")
    sections += ["\n## Rust standard library and compiler support\n", "Release toolchain: `" + version + "`; target: `" + args.target + "`.\n", "The following attribution index preserves the text of the toolchain's `share/doc/rust/COPYRIGHT-library.html`. Its embedded full licence texts are referenced in the final section. This upstream inventory is broader than the set of standard-library code selected by the linker.\n"]

    def substitute(match):
        return html.escape(notices.link(html.unescape(match.group(1))))

    report = re.sub(r"<pre[^>]*>(.*?)</pre>", substitute, report, flags=re.S)
    parser = PlainReport()
    parser.feed(report)
    sections.append(parser.text())
    sections.append("\n### Standard-library licence texts\n")
    license_directory = args.sysroot / "share" / "doc" / "rust" / "licenses"
    for path in sorted(license_directory.glob("*.txt")):
        sections.append("- `" + path.name + "`: " + notices.link(read(path)))

    if args.target.endswith("-windows-gnu"):
        gcc = args.sysroot / "lib" / "rustlib" / args.target / "bin" / "self-contained" / "x86_64-w64-mingw32-gcc.exe"
        details = run(str(gcc), "-v")
        if not re.search(r"rt_v12(?:-|/)", details):
            raise ValueError("GNU runtime is not the reviewed MinGW-w64 v12 build; update embedded notices before publishing")
        gcc_version = next(line for line in details.splitlines() if line.startswith("gcc version "))
        doc = args.sysroot / "share" / "doc"
        for text, expected in ((MINGW_RUNTIME, MINGW_RUNTIME_SHA256), (WINPTHREADS, WINPTHREADS_SHA256)):
            if hashlib.sha256(text.encode("utf-8")).hexdigest() != expected:
                raise ValueError("Embedded MinGW notice checksum mismatch")
        base = "https://github.com/mingw-w64/mingw-w64/blob/v" + MINGW_VERSION + "/"
        sections += [
            "\n## GNU Windows runtime\n",
            "The GNU release uses the Rust-distributed `" + gcc_version + "` toolchain, built with the MinGW-w64 v12 runtime. GCC itself and the development toolchain are not distributed with NestDesktop. The executable may contain statically linked runtime/startup code. The toolchain's runtime exception and upstream runtime notices are included here.\n",
            "- GCC GNU GPL version 3: " + notices.link(read(doc / "COPYING3")),
            "- GCC Runtime Library Exception version 3.1: " + notices.link(read(doc / "COPYING.RUNTIME")),
            "- [MinGW-w64 runtime notices](" + base + "COPYING.MinGW-w64-runtime/COPYING.MinGW-w64-runtime.txt): " + notices.link(MINGW_RUNTIME),
            "- [winpthreads notices](" + base + "mingw-w64-libraries/winpthreads/COPYING): " + notices.link(WINPTHREADS),
            "\nThese are third-party runtime terms and notices; they are not the licence for NestDesktop's own source code.\n",
        ]
    sections += ["\n## Full notice texts\n", "The text below is copied from the cached upstream licence files and toolchain report, with line endings normalized.\n"]
    for digest, text in notices.items.items():
        key = "notice-" + digest[:16]
        sections.append('<a id="' + key + '"></a>\n\n### ' + key + "\n\n````text\n" + text + "````\n")
    return "\n".join(sections).rstrip() + "\n", len(crates), len(notices.items)


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lockfile", type=Path, default=root / "Cargo.lock")
    parser.add_argument("--cargo-home", type=Path, default=Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")))
    parser.add_argument("--rustc", default="rustc", help="rustc command or executable path")
    parser.add_argument("--sysroot", type=Path, help="Rust sysroot containing its library copyright report")
    parser.add_argument("--target", default="x86_64-pc-windows-gnu")
    parser.add_argument("--output", type=Path, default=root / "THIRD_PARTY_NOTICES.md")
    parser.add_argument("--check", action="store_true", help="fail if the checked-in notice file would change")
    args = parser.parse_args()
    if args.sysroot is None:
        args.sysroot = Path(run(args.rustc, "--print", "sysroot"))
    content, crates, texts = generate(args)
    if args.check:
        if not args.output.is_file() or read(args.output) != content:
            raise ValueError("Third-party notices are out of date; regenerate and review them")
        print("Third-party notices match: {} locked crates, {} unique texts".format(crates, texts))
    else:
        with args.output.open("w", encoding="utf-8", newline="\n") as output:
            output.write(content)
        print("Generated notices: {} locked crates, {} unique texts, {} bytes".format(crates, texts, len(content.encode("utf-8"))))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
