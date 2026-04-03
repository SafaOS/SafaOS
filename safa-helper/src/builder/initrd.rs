use std::{
    fs::OpenOptions,
    io::{self, BufReader, Write},
    path::{Path, PathBuf},
};

use tempfile::{NamedTempFile, TempDir};

use crate::{ROOT_REPO_PATH, log, utils::https_get_write};

/// The directory where the raw ramdisk files should be put to be copied over.
pub const RAMDISK_INCLUDE_DIR: &str = "ramdisk-include";
pub const FONTS_DIR: &str = "fonts";

pub fn download_dejavu_fonts(out_dir: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
    const URL: &str = "https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/dejavu-fonts-ttf-2.37.zip";
    let out_dir = out_dir.as_ref();

    let work_dir = TempDir::new()?;
    let mut tmp = NamedTempFile::new()?;

    log!("Downloading Dejavu Fonts");
    https_get_write(
        URL,
        &[
            "Accept: application/vnd.github+json",
            "X-GitHub-Api-Version: 2022-11-28",
        ],
        |data| Ok(tmp.write(data).expect("Failed to write to download file")),
    )?;

    log!(
        "Dejavu Fonts downloaded, extracting from: {}...",
        tmp.path().display()
    );

    let reader = BufReader::new(tmp);
    zip::ZipArchive::new(reader)
        .expect("Failed to create zip archive")
        .extract(work_dir.path())
        .expect("Failed to unpack zip file");

    let ttf_dir_path = work_dir.path().join("dejavu-fonts-ttf-2.37").join("ttf");
    log!(
        "Dejavu Fonts extracted, copying from: {}...",
        ttf_dir_path.display()
    );
    let ttf_dir = std::fs::read_dir(ttf_dir_path)?;
    let ttf_fonts = ttf_dir.filter_map(|e| e.ok()).filter_map(|e| {
        e.path()
            .extension()
            .is_some_and(|e| e == "ttf")
            .then(|| e.path())
    });

    let mut result_fonts = Vec::with_capacity(ttf_fonts.size_hint().0);
    for p in ttf_fonts {
        let out = out_dir.join(p.file_name().unwrap());
        std::fs::copy(p, &out)?;
        result_fonts.push(out);
    }

    Ok(result_fonts)
}

pub fn download_emoji_fonts(out_dir: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
    const URL: &str =
        "https://github.com/googlefonts/noto-emoji/raw/refs/heads/main/fonts/NotoColorEmoji.ttf";

    log!("Downloading NotoColorEmoji: {URL}");
    let mut tmp = NamedTempFile::new()?;
    https_get_write(
        URL,
        &[
            "Accept: application/vnd.github+json",
            "X-GitHub-Api-Version: 2022-11-28",
        ],
        |data| Ok(tmp.write(data).expect("Failed to write to download file")),
    )?;

    let out_path = out_dir.as_ref().join("NotoColorEmoji.ttf");
    log!(
        "Copying from: {} to {}",
        tmp.path().display(),
        out_path.display()
    );
    std::fs::copy(tmp.path(), &out_path)?;
    Ok(vec![out_path])
}

#[repr(u8)]
// We want Sans => Sans Mono => Serif => Sans Condensed => Other
enum Family {
    Emoji,
    Sans,
    SansMono,
    Serif,
    Other,
}

fn fonts_dir() -> PathBuf {
    ROOT_REPO_PATH.join(RAMDISK_INCLUDE_DIR).join(FONTS_DIR)
}

pub fn get_fonts() -> io::Result<()> {
    let path = fonts_dir();
    _ = std::fs::remove_dir_all(&path);

    log!("Initializing Fonts");
    std::fs::create_dir(&path)?;

    let mut fonts = download_dejavu_fonts(&path)?;
    match download_emoji_fonts(&path) {
        Ok(mut e_fonts) => fonts.append(&mut e_fonts),
        Err(e) => {
            crate::log!("Failed to retrieve emoji fonts: {}", e);
        }
    }

    for f in &fonts {
        log!("Got Font: {}", f.display());
    }

    init_fonts_from(&path, fonts.into_iter()).expect("Failed to generate fontlist");
    Ok(())
}

fn init_fonts_from(fonts_dir: &Path, paths: impl Iterator<Item = PathBuf>) -> io::Result<()> {
    let fonts_list_path = fonts_dir.join("fontlist");
    _ = std::fs::remove_file(&fonts_list_path);

    let mut fonts: [Vec<String>; Family::Other as usize + 1] =
        [const { Vec::new() }; Family::Other as usize + 1];
    let mut final_fonts: Vec<String> = Vec::new();

    for font_path in paths {
        if !font_path.starts_with(fonts_dir) {
            log!(
                "Error: Font {}, doesn't belong to: {}",
                font_path.display(),
                fonts_dir.display()
            );
            continue;
        }

        let file_name = font_path
            .file_name()
            .expect("Font isn't a file")
            .to_str()
            .expect("Font file name isn't a UTF8 str");

        let family;
        if file_name.contains("Emoji") {
            family = Family::Emoji;
        } else if file_name.contains("Sans") && file_name.contains("Mono") {
            family = Family::SansMono;
        } else if file_name.contains("Sans") && !file_name.contains("Condensed") {
            family = Family::Sans;
        } else if file_name.contains("Serif") && !file_name.contains("Condensed") {
            family = Family::Serif;
        } else {
            family = Family::Other;
        }

        fonts[family as usize].push(file_name.to_string());
    }

    for list in &mut fonts {
        list.sort_by_key(|s| s.len());
    }

    final_fonts.append(&mut fonts[Family::Sans as usize]);
    final_fonts.append(&mut fonts[Family::SansMono as usize]);
    final_fonts.append(&mut fonts[Family::Emoji as usize]);
    final_fonts.append(&mut fonts[Family::Serif as usize]);
    final_fonts.append(&mut fonts[Family::Other as usize]);

    let mut font_list = OpenOptions::new()
        .write(true)
        .create(true)
        .open(fonts_list_path)?;
    for f in final_fonts {
        writeln!(font_list, "{f}")?;
    }
    Ok(())
}

pub fn init_fonts() -> io::Result<()> {
    let fonts_dir = fonts_dir();

    let dir = std::fs::read_dir(&fonts_dir)?;
    init_fonts_from(&fonts_dir, dir.filter_map(|e| e.ok()).map(|e| e.path()))
}
