//! Native save-handle components: actual Windows sharing and held identity.
//! This does not claim to run the full HTTP save adapter or Unix branches.
use super::*;
use std::{
    io::{Seek, SeekFrom},
    os::windows::io::AsRawHandle,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
};

fn identity(file: &File) -> (u32, u32, u32) {
    // SAFETY: zero initializes an output-only POD and this file owns its handle.
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the native output pointer and file handle remain live for the call.
    assert_ne!(
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) },
        0
    );
    (
        information.dwVolumeSerialNumber,
        information.nFileIndexHigh,
        information.nFileIndexLow,
    )
}
fn sharing_refusal(operation: &str, result: io::Result<()>) {
    let error = result.expect_err(operation);
    assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error:?}");
}
fn read_held(file: &mut File) -> Vec<u8> {
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    bytes
}

#[test]
fn windows_parent_pin_before_leaf_open_never_redirects_to_a_recreated_directory() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("validated-parent");
    let moved = root.path().join("retained-original-parent");
    std::fs::create_dir(&parent).unwrap();
    let selected = parent.join("fixture.txt");
    std::fs::write(&selected, b"validated original inode").unwrap();
    let original_identity = identity(&File::open(&selected).unwrap());
    let directory = Dir::open(&parent).unwrap();

    match std::fs::rename(&parent, &moved) {
        Err(error) => {
            assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error:?}");
            assert!(!moved.exists());
            let mut file = directory.open_file("fixture.txt", true).unwrap();
            assert_eq!(identity(&file), original_identity);
            assert_eq!(read_held(&mut file), b"validated original inode");
            drop(file);
            drop(directory);
            // The filesystem operation itself is valid once the directory's
            // native capability is released, without changing any ACL.
            std::fs::rename(&parent, &moved).unwrap();
            std::fs::create_dir(&parent).unwrap();
            std::fs::write(&selected, b"foreign replacement inode").unwrap();
            let replacement = Dir::open(&parent).unwrap();
            let mut file = replacement.open_file("fixture.txt", true).unwrap();
            assert_ne!(identity(&file), original_identity);
            assert_eq!(read_held(&mut file), b"foreign replacement inode");
        }
        Ok(()) => {
            // Reuse the old spelling with an independent plain directory. No
            // junction, symlink, ACL change or external path is involved.
            std::fs::create_dir(&parent).unwrap();
            std::fs::write(&selected, b"foreign replacement inode").unwrap();
            let replacement_identity = identity(&File::open(&selected).unwrap());
            assert_ne!(replacement_identity, original_identity);
            let observed = match directory.open_file("fixture.txt", true) {
                Err(_) => None,
                Ok(mut file) => {
                    let observed_identity = identity(&file);
                    let observed_bytes = read_held(&mut file);
                    // Only a genuinely retained original object may receive a
                    // write. A foreign handle is observed without modifying it.
                    if observed_identity == original_identity {
                        file.seek(SeekFrom::Start(0)).unwrap();
                        file.write_all(b"saved original").unwrap();
                        file.set_len(b"saved original".len() as u64).unwrap();
                        file.sync_all().unwrap();
                    }
                    Some((observed_identity, observed_bytes))
                }
            };
            drop(directory);
            assert_eq!(
                std::fs::read(&selected).unwrap(),
                b"foreign replacement inode"
            );
            assert_eq!(
                identity(&File::open(&selected).unwrap()),
                replacement_identity
            );
            let original_bytes = std::fs::read(moved.join("fixture.txt")).unwrap();
            match observed {
                None => assert_eq!(original_bytes, b"validated original inode"),
                Some((observed_identity, observed_bytes)) => {
                    // This is the negative control on the old Windows code:
                    // a path-based reopen reads the independent replacement.
                    assert_eq!(
                        observed_identity, original_identity,
                        "the held parent resolved a foreign inode before leaf validation"
                    );
                    assert_eq!(observed_bytes, b"validated original inode");
                    assert_eq!(original_bytes, b"saved original");
                }
            }
        }
    }
}

#[test]
fn windows_save_pins_refuse_parent_ancestor_and_leaf_replacement_until_handles_drop() {
    let root = tempfile::tempdir().unwrap();
    let ancestor = root.path().join("ancestor");
    let parent = ancestor.join("selected-parent");
    let moved_parent = ancestor.join("moved-parent");
    let moved_ancestor = root.path().join("moved-ancestor");
    let foreign = root.path().join("foreign");
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::create_dir(&foreign).unwrap();
    let target = parent.join("fixture.txt");
    let moved_target = parent.join("moved-fixture.txt");
    let competitor = foreign.join("replacement.txt");
    let sentinel = foreign.join("sentinel.txt");
    std::fs::write(&target, b"selected original").unwrap();
    std::fs::write(&competitor, b"independent replacement").unwrap();
    std::fs::write(&sentinel, b"foreign sentinel").unwrap();

    // These are precisely the native capabilities consumed by files-save:
    // pin every parent, open a regular writable leaf, then lock the handle.
    let directory = Dir::open(&parent).unwrap();
    let mut file = directory.open_file("fixture.txt", true).unwrap();
    file.try_lock().unwrap();
    let original_identity = identity(&file);
    let original_mtime = file.metadata().unwrap().modified().unwrap();

    sharing_refusal(
        "rename pinned parent during locked save",
        std::fs::rename(&parent, &moved_parent),
    );
    sharing_refusal(
        "rename pinned ancestor during locked save",
        std::fs::rename(&ancestor, &moved_ancestor),
    );
    sharing_refusal(
        "rename locked leaf during save",
        std::fs::rename(&target, &moved_target),
    );
    sharing_refusal(
        "replace locked leaf during save",
        std::fs::rename(&competitor, &target),
    );
    sharing_refusal(
        "delete locked leaf during save",
        std::fs::remove_file(&target),
    );
    assert!(!moved_parent.exists() && !moved_ancestor.exists() && !moved_target.exists());
    assert_eq!(identity(&file), original_identity);
    assert_eq!(file.metadata().unwrap().modified().unwrap(), original_mtime);
    assert_eq!(read_held(&mut file), b"selected original");
    assert_eq!(
        std::fs::read(&competitor).unwrap(),
        b"independent replacement"
    );

    let content = b"saved through the validated handle";
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(content).unwrap();
    file.set_len(content.len() as u64).unwrap();
    file.sync_all().unwrap();
    assert_eq!(identity(&file), original_identity);
    assert_eq!(read_held(&mut file), content);
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"foreign sentinel");
    assert_eq!(
        std::fs::read(&competitor).unwrap(),
        b"independent replacement"
    );

    // Positive controls prove the same fixture operations are possible once
    // each particular native pin is released, without ACL or attribute edits.
    drop(file);
    assert_eq!(std::fs::read(&target).unwrap(), content);
    std::fs::rename(&competitor, &target).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"independent replacement");
    let replacement = directory.open_file("fixture.txt", true).unwrap();
    assert_ne!(identity(&replacement), original_identity);
    drop(replacement);
    sharing_refusal(
        "rename pinned parent after leaf handle drops",
        std::fs::rename(&parent, &moved_parent),
    );
    drop(directory);
    std::fs::rename(&parent, &moved_parent).unwrap();
    std::fs::rename(&ancestor, &moved_ancestor).unwrap();
    assert_eq!(
        std::fs::read(moved_ancestor.join("moved-parent").join("fixture.txt")).unwrap(),
        b"independent replacement"
    );
    assert_eq!(std::fs::read(sentinel).unwrap(), b"foreign sentinel");
}
