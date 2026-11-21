//! Generates the `.res` linked into non-release Windows builds. The release
//! pipeline compiles `windows_resource_script` (scripts/rust-candidate-ci.py)
//! output with SDK rc.exe instead; both use icon group 1 and manifest 1.
use std::io;

/// One .res resource entry: header then DWORD-aligned data.
fn res_entry(output: &mut Vec<u8>, kind: u16, name: u16, language: u16, data: &[u8]) {
    while !output.len().is_multiple_of(4) {
        output.push(0);
    }
    output.extend_from_slice(&(data.len() as u32).to_le_bytes());
    output.extend_from_slice(&32u32.to_le_bytes()); // HeaderSize
    output.extend_from_slice(&0xFFFFu16.to_le_bytes());
    output.extend_from_slice(&kind.to_le_bytes());
    output.extend_from_slice(&0xFFFFu16.to_le_bytes());
    output.extend_from_slice(&name.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    output.extend_from_slice(&0x1030u16.to_le_bytes()); // MOVEABLE|PURE|DISCARDABLE
    output.extend_from_slice(&language.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes()); // Version
    output.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    output.extend_from_slice(data);
}

/// `.res` bytes carrying the ICO as RT_GROUP_ICON 1 + RT_ICON entries and the
/// manifest as RT_MANIFEST 1, so dev/candidate binaries look like release ones.
pub fn dev_windows_resource(icon: &[u8], manifest: &[u8]) -> io::Result<Vec<u8>> {
    const RT_ICON: u16 = 3;
    const RT_GROUP_ICON: u16 = 14;
    const RT_MANIFEST: u16 = 24;
    if icon.len() < 6 || icon[..4] != [0, 0, 1, 0] {
        return Err(io::Error::other(
            "assets/many-ai-cli.ico is not an ICO file",
        ));
    }
    let count = usize::from(u16::from_le_bytes([icon[4], icon[5]]));
    if count == 0 || icon.len() < 6 + count * 16 {
        return Err(io::Error::other(
            "assets/many-ai-cli.ico directory is truncated",
        ));
    }
    // A .res file opens with an empty resource marker.
    let mut output = Vec::new();
    res_entry(&mut output, 0, 0, 0, &[]);
    // The ICO header's reserved/type/count layout matches GRPICONDIR; each
    // 16-byte dir entry becomes a 14-byte group entry (offset replaced by nId).
    let mut group = icon[..6].to_vec();
    for index in 0..count {
        let base = 6 + index * 16;
        let entry = &icon[base..base + 16];
        let size = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        let valid = offset
            .checked_add(size)
            .is_some_and(|end| size > 0 && end <= icon.len());
        if !valid {
            return Err(io::Error::other(
                "assets/many-ai-cli.ico image is out of bounds",
            ));
        }
        let id = u16::try_from(index + 1).expect("ICO image count fits u16");
        res_entry(&mut output, RT_ICON, id, 0, &icon[offset..offset + size]);
        group.extend_from_slice(&entry[..12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    res_entry(&mut output, RT_GROUP_ICON, 1, 0, &group);
    res_entry(&mut output, RT_MANIFEST, 1, 0, manifest);
    Ok(output)
}
