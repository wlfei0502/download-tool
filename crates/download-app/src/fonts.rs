//! Load a system CJK font so Chinese UI labels render (egui's default font has no CJK glyphs).

use egui::{FontData, FontDefinitions, FontFamily};

/// Candidate paths for a single-file Chinese font (TTF/OTF preferred over TTC).
fn cjk_font_candidates() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();

    #[cfg(target_os = "macos")]
    {
        paths.push(std::path::PathBuf::from(
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        ));
        paths.push(std::path::PathBuf::from(
            "/System/Library/Fonts/Supplemental/NISC18030.ttf",
        ));
        paths.push(std::path::PathBuf::from(
            "/Library/Fonts/Arial Unicode.ttf",
        ));
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(windir) = std::env::var("WINDIR") {
            let fonts = std::path::Path::new(&windir).join("Fonts");
            paths.push(fonts.join("msyh.ttc"));
            paths.push(fonts.join("msyh.ttf"));
            paths.push(fonts.join("simhei.ttf"));
            paths.push(fonts.join("simsun.ttc"));
            paths.push(fonts.join("msyhbd.ttc"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        paths.extend(
            [
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf",
                "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
                "/usr/share/fonts/truetype/arphic/uming.ttc",
                "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            ]
            .into_iter()
            .map(std::path::PathBuf::from),
        );
    }

    paths
}

fn load_cjk_font_data() -> Option<FontData> {
    for path in cjk_font_candidates() {
        match std::fs::read(&path) {
            Ok(bytes) if !bytes.is_empty() => {
                eprintln!("using CJK font {}", path.display());
                return Some(FontData::from_owned(bytes));
            }
            Ok(_) => {}
            Err(err) => {
                eprintln!("skip CJK font {}: {err}", path.display());
            }
        }
    }
    None
}

/// Install a CJK fallback font on the egui context. Safe to call once at startup.
pub fn configure_fonts(ctx: &egui::Context) {
    let Some(cjk) = load_cjk_font_data() else {
        eprintln!(
            "no CJK system font found; Chinese UI may show as boxes. \
             Install a Noto Sans CJK / 微软雅黑 font, or place Arial Unicode on macOS."
        );
        return;
    };

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("cjk".into(), cjk.into());

    // Keep default Latin fonts first; fall back to CJK for missing glyphs.
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("cjk".to_owned());
    }

    ctx.set_fonts(fonts);
}
