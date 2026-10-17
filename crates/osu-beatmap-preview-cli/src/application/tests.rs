#![cfg(test)]
//! 来源测试只使用临时文件和假下载端口，禁止实时网络与认证依赖。
use super::*;
use crate::application::request::RenderRequest;
use crate::download::osr::Lookup;
use std::{cell::RefCell, io::Write};
use zip::write::SimpleFileOptions;

#[test]
fn checksum_uses_original_bytes_not_normalized_text() {
    let original = b"a\r\nb\r\n";
    let checksum = format!("{:x}", md5::compute(original));
    assert!(check_md5(original, &checksum).is_ok());
    assert!(check_md5(b"a\nb\n", &checksum).is_err());
}

#[test]
fn local_resources_take_priority_and_mismatch_does_not_download() {
    let fixture = Fixture::new();
    let download = fixture.downloads(None);
    let request = fixture.request(true, true);
    let (osu, osz, bytes) = resolve_resources(&request, &fixture.checksum, &download).unwrap();
    assert_eq!(osu, fixture.osu);
    assert_eq!(osz, Some(fixture.osz.clone()));
    assert_eq!(bytes, fixture.bytes);
    assert!(download.calls.borrow().is_empty());
    assert!(resolve_resources(&request, &"0".repeat(32), &download).is_err());
    assert!(download.calls.borrow().is_empty());
    let request = fixture.request(false, true);
    assert_eq!(
        resolve_resources(&request, &fixture.checksum, &download)
            .unwrap()
            .2,
        fixture.bytes
    );
}

#[test]
fn missing_resources_query_checksum_and_download_only_what_is_needed() {
    let fixture = Fixture::new();
    for (local_osu, expected) in [
        (true, vec!["lookup", "archive"]),
        (false, vec!["lookup", "beatmap", "archive"]),
    ] {
        let download = fixture.downloads(None);
        let request = fixture.request(local_osu, false);
        let resolved = resolve_resources(&request, &fixture.checksum, &download).unwrap();
        assert_eq!(resolved.2, fixture.bytes);
        assert_eq!(*download.calls.borrow(), expected);
    }
}

#[test]
fn every_download_failure_and_wrong_checksum_returns_error() {
    let fixture = Fixture::new();
    let request = fixture.request(false, false);
    for operation in ["lookup", "beatmap", "archive"] {
        let download = fixture.downloads(Some(operation));
        assert!(resolve_resources(&request, &fixture.checksum, &download)
            .unwrap_err()
            .to_string()
            .contains(operation));
    }
    let mut download = fixture.downloads(None);
    download.checksum = "0".repeat(32);
    assert!(resolve_resources(&request, &fixture.checksum, &download).is_err());
    assert_eq!(*download.calls.borrow(), ["lookup"]);
    std::fs::write(&fixture.osu, b"wrong beatmap").unwrap();
    let download = fixture.downloads(None);
    assert!(resolve_resources(&request, &fixture.checksum, &download).is_err());
    assert_eq!(*download.calls.borrow(), ["lookup", "beatmap"]);
}

#[test]
fn osz_must_contain_matching_top_level_map_and_nonempty_song() {
    let fixture = Fixture::new();
    let beatmap = parse_beatmap_bytes(&fixture.bytes).unwrap();
    assert!(validate_archive_audio(&fixture.osz, &beatmap).is_ok());
    fixture.archive(&[
        ("wrong.osu", b"wrong"),
        ("nested/map.osu", &fixture.bytes),
        ("song.wav", b"audio"),
    ]);
    assert!(find_md5(&fixture.osz, &fixture.checksum).is_err());
    fixture.archive(&[("map.osu", &fixture.bytes)]);
    assert!(validate_archive_audio(&fixture.osz, &beatmap).is_err());
    fixture.archive(&[("map.osu", &fixture.bytes), ("song.wav", b"")]);
    assert!(validate_archive_audio(&fixture.osz, &beatmap).is_err());
}

#[test]
fn gif_uses_only_local_osu_or_downloads_osu_without_opening_archive() {
    let fixture = Fixture::new();
    for local in [true, false] {
        let mut request = fixture.request(local, true);
        request.output.format = Some("gif".into());
        // 非 ZIP 文件可证明 GIF 没有尝试解包或读取音频、背景。
        std::fs::write(&fixture.osz, b"not a zip archive").unwrap();
        let downloads = fixture.downloads(Some("archive"));
        let (osu, archive, bytes) =
            resolve_resources(&request, &fixture.checksum, &downloads).unwrap();
        assert_eq!(osu, fixture.osu);
        assert!(archive.is_none());
        assert_eq!(bytes, fixture.bytes);
        assert_eq!(
            *downloads.calls.borrow(),
            if local {
                vec![]
            } else {
                vec!["lookup", "beatmap"]
            }
        );
    }
    let mut request = fixture.request(true, false);
    request.output.format = Some("gif".into());
    let downloads = fixture.downloads(None);
    assert!(resolve_resources(&request, &"0".repeat(32), &downloads).is_err());
    assert!(downloads.calls.borrow().is_empty());
    request.source.osu_file = None;
    let mut downloads = fixture.downloads(None);
    downloads.checksum = "0".repeat(32);
    assert!(resolve_resources(&request, &fixture.checksum, &downloads).is_err());
    let downloads = fixture.downloads(None);
    std::fs::write(&fixture.osu, b"mismatched osu").unwrap();
    assert!(resolve_resources(&request, &fixture.checksum, &downloads).is_err());
}

#[test]
fn local_gif_load_does_not_require_audio_or_official_credentials() {
    let fixture = Fixture::new();
    let mut request = fixture.request(true, false);
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../osu-beatmap-preview-web/test/fixtures");
    request.source.osu_file = Some(
        root.join("replay-standard.osu")
            .to_string_lossy()
            .into_owned(),
    );
    request.kind = RequestKind::Osr {
        source: crate::ReplaySource::File(
            root.join("replay-standard.osr")
                .to_string_lossy()
                .into_owned(),
        ),
    };
    request.output.format = Some("gif".into());
    let deadline = RequestDeadline::new(
        std::time::Instant::now(),
        "gif",
        std::time::Duration::from_secs(5),
    );
    let source = load(&request, &fixture.directory, &deadline).unwrap();
    assert!(source.osz_path.is_none());
    assert!(source.beatmap.general.get("AudioFilename").is_none());
}

struct FakeDownloads {
    checksum: String,
    osu: PathBuf,
    osz: PathBuf,
    calls: RefCell<Vec<&'static str>>,
    failure: Option<&'static str>,
}
impl FakeDownloads {
    fn call(&self, name: &'static str) -> Result<()> {
        self.calls.borrow_mut().push(name);
        if self.failure == Some(name) {
            Err(PreviewError::download(format!("fake {name} failed")))
        } else {
            Ok(())
        }
    }
}
impl Downloads for FakeDownloads {
    fn lookup(&self, checksum: &str) -> Result<Lookup> {
        self.call("lookup")?;
        assert_eq!(checksum.len(), 32);
        Ok(Lookup {
            id: 1,
            beatmapset_id: 2,
            checksum: self.checksum.clone(),
        })
    }
    fn beatmap(&self, _: &Lookup) -> Result<PathBuf> {
        self.call("beatmap")?;
        Ok(self.osu.clone())
    }
    fn archive(&self, _: &Lookup) -> Result<PathBuf> {
        self.call("archive")?;
        Ok(self.osz.clone())
    }
}
struct Fixture {
    directory: PathBuf,
    osu: PathBuf,
    osz: PathBuf,
    bytes: Vec<u8>,
    checksum: String,
}
impl Fixture {
    fn new() -> Self {
        static SEQUENCE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "osr-source-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let bytes = b"osu file format v14\n[General]\nMode:0\nAudioFilename:song.wav\n[Difficulty]\nCircleSize:5\nOverallDifficulty:5\nSliderMultiplier:1\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n100,100,1000,1,0\n".to_vec();
        let checksum = format!("{:x}", md5::compute(&bytes));
        let fixture = Self {
            osu: directory.join("map.osu"),
            osz: directory.join("map.osz"),
            directory,
            bytes,
            checksum,
        };
        std::fs::write(&fixture.osu, &fixture.bytes).unwrap();
        fixture.archive(&[
            ("other.osu", b"wrong"),
            ("map.osu", &fixture.bytes),
            ("song.wav", b"audio"),
        ]);
        fixture
    }
    fn archive(&self, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(&self.osz).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        for (name, bytes) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }
    fn request(&self, osu: bool, osz: bool) -> ValidatedRequest {
        let mut request = RenderRequest::new("");
        request.kind = RequestKind::Osr {
            source: crate::ReplaySource::File("test.osr".into()),
        };
        request.source.osu_file = osu.then(|| self.osu.to_string_lossy().into_owned());
        request.source.osz_file = osz.then(|| self.osz.to_string_lossy().into_owned());
        request.validate().unwrap()
    }
    fn downloads(&self, failure: Option<&'static str>) -> FakeDownloads {
        FakeDownloads {
            checksum: self.checksum.clone(),
            osu: self.osu.clone(),
            osz: self.osz.clone(),
            calls: RefCell::new(vec![]),
            failure,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
