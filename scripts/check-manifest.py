import struct, pathlib, sys, xml.etree.ElementTree as ET
base = pathlib.Path(__file__).resolve().parents[1]
def check(file):
    b=file.read_bytes(); pe=struct.unpack_from('<I',b,60)[0]; coff=pe+4
    count=struct.unpack_from('<H',b,coff+2)[0]; osz=struct.unpack_from('<H',b,coff+16)[0]; opt=coff+20
    dirs=opt+(112 if struct.unpack_from('<H',b,opt)[0]==0x20b else 96)
    rrva,rsize=struct.unpack_from('<II',b,dirs+16); assert rrva and rsize
    sections=[]
    for i in range(count):
        s=opt+osz+i*40; vs,va,rs,rp=struct.unpack_from('<IIII',b,s+8); sections.append((va,max(vs,rs),rp))
    def off(rva):
        for va,n,rp in sections:
            if va<=rva<va+n:return rp+rva-va
        raise AssertionError('Unmapped RVA')
    root=off(rrva)
    def entries(rel):
        named,ids=struct.unpack_from('<HH',b,root+rel+12)
        return [struct.unpack_from('<II',b,root+rel+16+8*i) for i in range(named+ids)]
    typee=next(e for e in entries(0) if e[0]==24); assert typee[1]&0x80000000
    ide=next(e for e in entries(typee[1]&0x7fffffff) if e[0]==1); assert ide[1]&0x80000000
    lang=next(e for e in entries(ide[1]&0x7fffffff) if e[0]==0x409); assert not(lang[1]&0x80000000)
    mrva,msz,cp,reserved=struct.unpack_from('<IIII',b,root+lang[1]); xml=b[off(mrva):off(mrva)+msz]
    assert xml==(base/'assets/app.manifest').read_bytes()
    tree=ET.fromstring(xml); ns={'v3':'urn:schemas-microsoft-com:asm.v3','c':'urn:schemas-microsoft-com:compatibility.v1','d':'http://schemas.microsoft.com/SMI/2016/WindowsSettings'}
    req=tree.find('.//v3:requestedExecutionLevel',ns); assert req.attrib=={'level':'asInvoker','uiAccess':'false'}
    osids={x.attrib['Id'] for x in tree.findall('.//c:supportedOS',ns)}
    assert '{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}' in osids and '{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}' in osids
    assert tree.find('.//d:dpiAwareness',ns).text=='PerMonitorV2'
    assert tree.find('.//d:longPathAware',ns).text=='true'
    print(f'{file.name}: embedded RT_MANIFEST/1/0x0409 verified ({msz} XML bytes, resource RVA 0x{rrva:x}, codepage {cp})')

if __name__ == "__main__":
    check(pathlib.Path(sys.argv[1]).resolve())
