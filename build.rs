//! Embed the manifest and multi-resolution application icon without rc.exe.
//!
//! A normal AMD64 COFF .rsrc object works with both GNU and MSVC linkers.
//! Each resource data RVA has an IMAGE_REL_AMD64_ADDR32NB relocation.

use std::{collections::BTreeMap, env, fs, path::PathBuf};

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn read16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn read32(bytes: &[u8], offset: usize) -> usize {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize
}
struct Resource {
    kind: u16,
    id: u16,
    data: Vec<u8>,
    codepage: u32,
}
fn icon_resources(ico: &[u8]) -> Vec<Resource> {
    assert!(ico.len() >= 6 && read16(ico, 0) == 0 && read16(ico, 2) == 1);
    let count = read16(ico, 4) as usize;
    assert!(count > 0 && count < 256 && ico.len() >= 6 + count * 16);
    let mut group = vec![0; 6 + count * 14];
    group[..6].copy_from_slice(&ico[..6]);
    let mut resources = Vec::new();
    for index in 0..count {
        let entry = 6 + index * 16;
        let size = read32(ico, entry + 8);
        let offset = read32(ico, entry + 12);
        assert!(offset >= 6 + count * 16 && size > 0 && offset + size <= ico.len());
        let id = 100 + index as u16;
        // GRPICONDIRENTRY replaces the ICO file offset with an RT_ICON ID.
        let destination = 6 + index * 14;
        group[destination..destination + 12].copy_from_slice(&ico[entry..entry + 12]);
        put16(&mut group, destination + 12, id);
        resources.push(Resource {
            kind: 3,
            id,
            data: ico[offset..offset + size].to_vec(),
            codepage: 0,
        });
    }
    resources.push(Resource {
        kind: 14,
        id: 1,
        data: group,
        codepage: 0,
    });
    resources
}
fn resource_object(resources: &[Resource]) -> Vec<u8> {
    let mut types = BTreeMap::<u16, BTreeMap<u16, usize>>::new();
    for (index, resource) in resources.iter().enumerate() {
        assert!(types
            .entry(resource.kind)
            .or_default()
            .insert(resource.id, index)
            .is_none());
    }
    // Root -> type -> ID -> en-US -> IMAGE_RESOURCE_DATA_ENTRY.
    let mut length = 16 + types.len() * 8;
    let mut type_offsets = BTreeMap::new();
    for (kind, entries) in &types {
        type_offsets.insert(*kind, length);
        length += 16 + entries.len() * 8;
    }
    let mut id_offsets = vec![0; resources.len()];
    for index in types.values().flat_map(|entries| entries.values()) {
        id_offsets[*index] = length;
        length += 24;
    }
    let mut data_offsets = vec![0; resources.len()];
    for offset in &mut data_offsets {
        *offset = length;
        length += 16;
    }
    let mut payload_offsets = Vec::new();
    for resource in resources {
        payload_offsets.push(length);
        length = (length + resource.data.len() + 3) & !3;
    }
    const RAW_OFFSET: usize = 20 + 40;
    const SYMBOL_SIZE: usize = 18;
    let relocation_offset = RAW_OFFSET + length;
    let symbol_offset = relocation_offset + resources.len() * 10;
    let mut object = vec![0; symbol_offset + SYMBOL_SIZE + 4];
    put16(&mut object, 0, 0x8664); // AMD64
    put16(&mut object, 2, 1);
    put32(&mut object, 8, symbol_offset as u32);
    put32(&mut object, 12, 1);
    object[20..28].copy_from_slice(b".rsrc\0\0\0");
    put32(&mut object, 36, length as u32);
    put32(&mut object, 40, RAW_OFFSET as u32);
    put32(&mut object, 44, relocation_offset as u32);
    put16(&mut object, 52, resources.len() as u16);
    put32(&mut object, 56, 0x4030_0040); // READ | ALIGN_4BYTES | INITIALIZED_DATA
    let resource_data = &mut object[RAW_OFFSET..RAW_OFFSET + length];
    put16(resource_data, 14, types.len() as u16);
    for (index, (kind, entries)) in types.iter().enumerate() {
        let directory = type_offsets[kind];
        put32(resource_data, 16 + index * 8, *kind as u32);
        put32(
            resource_data,
            20 + index * 8,
            0x8000_0000 | directory as u32,
        );
        put16(resource_data, directory + 14, entries.len() as u16);
        for (entry_index, (id, resource_index)) in entries.iter().enumerate() {
            let language = id_offsets[*resource_index];
            put32(resource_data, directory + 16 + entry_index * 8, *id as u32);
            put32(
                resource_data,
                directory + 20 + entry_index * 8,
                0x8000_0000 | language as u32,
            );
            put16(resource_data, language + 14, 1);
            put32(resource_data, language + 16, 0x0409);
            put32(
                resource_data,
                language + 20,
                data_offsets[*resource_index] as u32,
            );
        }
    }
    for (index, resource) in resources.iter().enumerate() {
        let data = data_offsets[index];
        let payload = payload_offsets[index];
        put32(resource_data, data, payload as u32);
        put32(resource_data, data + 4, resource.data.len() as u32);
        put32(resource_data, data + 8, resource.codepage);
        resource_data[payload..payload + resource.data.len()].copy_from_slice(&resource.data);
    }
    for (index, offset) in data_offsets.iter().enumerate() {
        let relocation = relocation_offset + index * 10;
        put32(&mut object, relocation, *offset as u32);
        put32(&mut object, relocation + 4, 0);
        put16(&mut object, relocation + 8, 0x0003); // ADDR32NB
    }
    object[symbol_offset..symbol_offset + 8].copy_from_slice(b".rsrc\0\0\0");
    put16(&mut object, symbol_offset + 12, 1);
    object[symbol_offset + 16] = 3; // static section symbol
    put32(&mut object, symbol_offset + SYMBOL_SIZE, 4);
    object
}
fn main() {
    for path in ["assets/app.manifest", "assets/nest-desktop.ico", "build.rs"] {
        println!("cargo:rerun-if-changed={path}");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    assert_eq!(
        env::var("CARGO_CFG_TARGET_ARCH").as_deref(),
        Ok("x86_64"),
        "The NestDesktop resource object currently supports Windows x64 builds."
    );
    let mut resources =
        icon_resources(&fs::read("assets/nest-desktop.ico").expect("read app icon"));
    resources.push(Resource {
        kind: 24, // RT_MANIFEST / 1 / en-US remains unchanged.
        id: 1,
        data: fs::read("assets/app.manifest").expect("read application manifest"),
        codepage: 65001,
    });
    let object = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"))
        .join("application-resources.obj");
    fs::write(&object, resource_object(&resources)).expect("write application resource object");
    println!("cargo:rustc-link-arg-bin=nest-desktop={}", object.display());
}
