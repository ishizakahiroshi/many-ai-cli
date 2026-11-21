use many_ai_cli::windows_resource::dev_windows_resource;

/// Parses the generated .res back into (kind, name, language, data) entries so
/// every header field lands where the linker expects it.
fn entries(bytes: &[u8]) -> Vec<(u16, u16, u16, &[u8])> {
    let mut entries = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        assert!(bytes.len() - pos >= 32, "entry header is truncated");
        let u32_at =
            |at: usize| u32::from_le_bytes(bytes[pos + at..pos + at + 4].try_into().unwrap());
        let u16_at =
            |at: usize| u16::from_le_bytes(bytes[pos + at..pos + at + 2].try_into().unwrap());
        let data_size = u32_at(0) as usize;
        let header_size = u32_at(4) as usize;
        assert_eq!(header_size, 32, "ordinal entries carry a 32-byte header");
        assert_eq!(u16_at(8), 0xFFFF, "resource type must stay ordinal");
        assert_eq!(u16_at(12), 0xFFFF, "resource name must stay ordinal");
        assert_eq!(u32_at(16), 0, "DataVersion");
        assert_eq!(u16_at(20), 0x1030, "MemoryFlags");
        assert_eq!(u32_at(24), 0, "Version");
        assert_eq!(u32_at(28), 0, "Characteristics");
        let (kind, name, language) = (u16_at(10), u16_at(14), u16_at(22));
        pos += header_size;
        assert!(bytes.len() - pos >= data_size, "entry data is truncated");
        entries.push((kind, name, language, &bytes[pos..pos + data_size]));
        pos += data_size.next_multiple_of(4);
    }
    entries
}

fn synthetic_ico(images: &[[u8; 3]]) -> Vec<u8> {
    let mut icon = vec![0, 0, 1, 0];
    icon.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + images.len() * 16;
    let mut payload = Vec::new();
    for image in images {
        let mut entry = vec![16u8, 16, 0, 0];
        entry.extend_from_slice(&1u16.to_le_bytes());
        entry.extend_from_slice(&32u16.to_le_bytes());
        entry.extend_from_slice(&(image.len() as u32).to_le_bytes());
        entry.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += image.len();
        icon.extend_from_slice(&entry);
    }
    for image in images {
        payload.extend_from_slice(image);
    }
    icon.extend_from_slice(&payload);
    icon
}

#[test]
fn dev_resource_carries_marker_grouped_icons_and_manifest() {
    let images: Vec<[u8; 3]> = vec![*b"aaa", *b"bbb", *b"ccc"];
    let icon = synthetic_ico(&images);
    let manifest = b"<assembly/>";
    let res = dev_windows_resource(&icon, manifest).unwrap();
    let entries = entries(&res);
    assert_eq!(entries.len(), images.len() + 3);
    let (kind, name, _, data) = entries[0];
    assert_eq!((kind, name, data.is_empty()), (0, 0, true), "empty marker");
    for (index, entry) in entries[1..1 + images.len()].iter().enumerate() {
        let (kind, name, language, data) = entry;
        assert_eq!(
            (*kind, *name, *language),
            (3, (index + 1) as u16, 0),
            "RT_ICON entry {index}"
        );
        assert_eq!(*data, images[index].as_slice());
    }
    let (kind, name, _, group) = entries[1 + images.len()];
    assert_eq!((kind, name), (14, 1), "RT_GROUP_ICON id 1");
    assert_eq!(&group[..6], &icon[..6], "group header mirrors ICO header");
    for index in 0..images.len() {
        let base = 6 + index * 14;
        let entry = &group[base..base + 14];
        assert_eq!(
            &entry[..12],
            &icon[6 + index * 16..6 + index * 16 + 12],
            "group entry {index} keeps the ICO fields"
        );
        assert_eq!(
            u16::from_le_bytes(entry[12..14].try_into().unwrap()),
            (index + 1) as u16,
            "group entry {index} nId"
        );
    }
    let (kind, name, _, data) = entries[2 + images.len()];
    assert_eq!(
        (kind, name, data),
        (24, 1, &manifest[..]),
        "RT_MANIFEST id 1"
    );
}

#[test]
fn dev_resource_rejects_malformed_icons() {
    let manifest = b"<assembly/>";
    assert!(dev_windows_resource(b"not-an-ico", manifest).is_err());
    assert!(dev_windows_resource(&[0, 0, 1, 0, 0, 0], manifest).is_err());
    let mut truncated = synthetic_ico(&[*b"aaa"]);
    truncated.truncate(10);
    assert!(dev_windows_resource(&truncated, manifest).is_err());
    let mut dangling = synthetic_ico(&[*b"aaa"]);
    dangling[6 + 12..6 + 16].copy_from_slice(&9999u32.to_le_bytes());
    assert!(dev_windows_resource(&dangling, manifest).is_err());
}
