//! Fixed Go download definitions; no inferred URL, hash or binary platform.
use serde::Serialize;
// v1.8.6 crashed on recordings shorter than half a frame (the reflective
// padding in log_mel_spectrogram read past the input). The fix is upstream
// PR #3956; v1.9.5 contains it. Upstream attaches the Windows binaries of that
// commit (d1be6fde) to the `b5454` release, not to the `v1.9.5` one.
pub const RELEASE: &str = "v1.9.5";
pub const DEFAULT_MODEL: &str = "small";
pub const EXTRA_ROOM: u64 = 256 * 1024 * 1024;
pub struct Binary {
    pub version: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size_bytes: u64,
    pub archive: &'static str,
    pub server_names: &'static [&'static str],
    pub keep: &'static [&'static str],
    pub runtime: &'static str,
}
pub const WINDOWS: Binary = Binary {
    version: RELEASE,
    url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-x64.zip",
    sha256: "6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb",
    size_bytes: 8928640,
    archive: "zip",
    server_names: &["whisper-server.exe", "server.exe"],
    // ggml.dll picks one CPU backend at run time from the `ggml-cpu-*.dll`
    // beside the server, so every variant in the archive is kept. An older
    // `ggml-cpu.dll` left by v1.8.6 is only a fallback when no variant loads.
    keep: &[
        "whisper-server.exe",
        "whisper.dll",
        "ggml.dll",
        "ggml-base.dll",
        "ggml-cpu-alderlake.dll",
        "ggml-cpu-cannonlake.dll",
        "ggml-cpu-cascadelake.dll",
        "ggml-cpu-haswell.dll",
        "ggml-cpu-icelake.dll",
        "ggml-cpu-sandybridge.dll",
        "ggml-cpu-skylakex.dll",
        "ggml-cpu-sse42.dll",
        "ggml-cpu-x64.dll",
    ],
    runtime: "windows-amd64",
};
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Model {
    pub id: &'static str,
    pub label: &'static str,
    pub file_name: &'static str,
    #[serde(skip)]
    pub url: &'static str,
    pub size_bytes: u64,
    pub quality: &'static str,
    pub sha256: &'static str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub default: bool,
    pub hash_checked: bool,
}
pub const MODELS: &[Model] = &[
    Model {
        id: "small",
        label: "Small (recommended)",
        file_name: "ggml-small.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        size_bytes: 488 * 1024 * 1024,
        quality: "fast on ordinary CPUs (2-3s per utterance), may misspell technical terms",
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        default: true,
        hash_checked: true,
    },
    Model {
        id: "large-v3-turbo-q5_0",
        label: "Large v3 Turbo Q5_0",
        file_name: "ggml-large-v3-turbo-q5_0.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        size_bytes: 574 * 1024 * 1024,
        quality: "best accuracy for Japanese/English; needs a fast multi-core CPU or GPU server",
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        default: false,
        hash_checked: true,
    },
    Model {
        id: "tiny-q5_1",
        label: "Tiny Q5_1",
        file_name: "ggml-tiny-q5_1.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin",
        size_bytes: 15 * 1024 * 1024,
        quality: "smoke test / very low resource",
        sha256: "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7",
        default: false,
        hash_checked: true,
    },
];
pub fn binary(platform: &str, arch: &str) -> Option<&'static Binary> {
    (platform == "windows" && arch == "amd64").then_some(&WINDOWS)
}
pub fn model(id: &str) -> Option<Model> {
    let id = id.trim();
    let id = if id.is_empty() { DEFAULT_MODEL } else { id };
    MODELS.iter().find(|model| model.id == id).copied()
}
pub const UNSUPPORTED: &str =
    "managed Whisper is not available on this platform; set an external Whisper server URL instead";
pub const MANUAL_ONLY: &str = "Managed install is not available on this platform. Use an external Whisper server URL, or run on a supported platform (Windows x64, or a Docker image with a bundled whisper-server).";
