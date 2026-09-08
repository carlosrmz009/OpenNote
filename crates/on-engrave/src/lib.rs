//! Putting the fingerings back on the page.
//!
//! Two outputs, from the same annotated document:
//!
//! * the score file itself — MusicXML or compressed `.mxl`, with a `<fingering>` on
//!   every note and everything else exactly as the engraver left it
//! * an engraved PDF or SVG, so there is something to print and put on the stand
//!
//! Engraving goes through [Verovio](https://www.verovio.org/), which reads the
//! annotated MusicXML directly, so the printed page and the score file cannot
//! disagree about what the fingerings are. Verovio emits SVG; PDF is produced from
//! that SVG, which keeps the notation as vectors rather than a rasterised image.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use on_score::{Fingering, MusicXmlDocument};

/// What to write out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageFormat {
    /// Vector PDF, one file for the whole piece.
    Pdf,
    /// Vector SVG, one file per page.
    Svg,
    /// Raster PNG, one file per page.
    Png,
}

impl PageFormat {
    /// Guess the format from a file extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        match path
            .extension()
            .and_then(|e| e.to_str())?
            .to_ascii_lowercase()
            .as_str()
        {
            "pdf" => Some(PageFormat::Pdf),
            "svg" => Some(PageFormat::Svg),
            "png" => Some(PageFormat::Png),
            _ => None,
        }
    }

    fn extension(self) -> &'static str {
        match self {
            PageFormat::Pdf => "pdf",
            PageFormat::Svg => "svg",
            PageFormat::Png => "png",
        }
    }
}

/// How the engraved page should look.
#[derive(Debug, Clone)]
pub struct EngraveOptions {
    /// Page width in tenths of a staff space, Verovio's unit. A4 portrait by default.
    pub page_width: i32,
    /// Page height in the same unit.
    pub page_height: i32,
    /// Overall scale, as a percentage.
    pub scale: i32,
    /// Whether to lay the music out across pages or as one continuous system.
    pub paginate: bool,
}

impl Default for EngraveOptions {
    fn default() -> Self {
        // A4 at Verovio's default 100 units per 10 mm.
        Self {
            page_width: 2100,
            page_height: 2970,
            scale: 40,
            paginate: true,
        }
    }
}

/// Write fingerings into a score and save it.
///
/// Returns how many notes were annotated.
pub fn annotate_and_save(
    document: &mut MusicXmlDocument,
    fingerings: &[Fingering],
    output: &Path,
) -> Result<usize> {
    let written = document.annotate(fingerings)?;
    let compressed = output
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mxl"));
    document.write(output, compressed)?;
    Ok(written)
}

/// Engrave a MusicXML file, returning the files written.
///
/// The input must already carry its fingerings; this reads the file rather than the
/// in-memory document so that what gets engraved is exactly what was saved.
pub fn engrave(
    musicxml: &Path,
    output: &Path,
    options: &EngraveOptions,
) -> Result<Vec<PathBuf>> {
    let format = PageFormat::from_path(output).ok_or_else(|| {
        anyhow!(
            "cannot tell what to engrave from the extension of {}; use .pdf, .svg or .png",
            output.display()
        )
    })?;

    let pages = render_pages(musicxml, options)?;
    if pages.is_empty() {
        bail!("Verovio produced no pages for {}", musicxml.display());
    }

    match format {
        PageFormat::Pdf => {
            write_pdf(&pages, output)?;
            Ok(vec![output.to_path_buf()])
        }
        PageFormat::Svg => {
            let paths = page_paths(output, pages.len(), format);
            for (svg, path) in pages.iter().zip(&paths) {
                std::fs::write(path, svg)
                    .with_context(|| format!("writing {}", path.display()))?;
            }
            Ok(paths)
        }
        PageFormat::Png => write_pngs(&pages, output),
    }
}

/// Render every page of a score to SVG.
pub fn render_pages(musicxml: &Path, options: &EngraveOptions) -> Result<Vec<String>> {
    let mut toolkit = verovioxide::Toolkit::new()
        .map_err(|e| anyhow!("starting Verovio: {e}"))?;

    let settings = format!(
        r#"{{"pageWidth":{},"pageHeight":{},"scale":{},"breaks":"{}","adjustPageHeight":{},"footer":"none","header":"none"}}"#,
        options.page_width,
        options.page_height,
        options.scale,
        if options.paginate { "auto" } else { "none" },
        if options.paginate { "false" } else { "true" },
    );
    let parsed: verovioxide::Options = serde_json::from_str(&settings)
        .with_context(|| format!("building Verovio options from {settings}"))?;
    toolkit
        .set_options(&parsed)
        .map_err(|e| anyhow!("configuring Verovio: {e}"))?;

    let data = std::fs::read_to_string(musicxml)
        .with_context(|| format!("reading {}", musicxml.display()))?;
    toolkit
        .load_data(&data)
        .map_err(|e| anyhow!("Verovio could not read {}: {e}", musicxml.display()))?;

    toolkit
        .render_all_pages()
        .map_err(|e| anyhow!("engraving {}: {e}", musicxml.display()))
}

/// Filenames for a multi-page output: `score.svg`, `score-2.svg`, and so on.
fn page_paths(output: &Path, count: usize, format: PageFormat) -> Vec<PathBuf> {
    if count <= 1 {
        return vec![output.to_path_buf()];
    }
    let stem = output
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("score");
    let parent = output.parent().unwrap_or(Path::new("."));
    (1..=count)
        .map(|i| {
            if i == 1 {
                output.to_path_buf()
            } else {
                parent.join(format!("{stem}-{i}.{}", format.extension()))
            }
        })
        .collect()
}

/// Convert the engraved SVG pages into one multi-page PDF.
///
/// Each page becomes a PDF form XObject drawn onto its own page, so the notation
/// stays as vector outlines and the file remains a normal, printable score rather
/// than a stack of images.
fn write_pdf(pages: &[String], output: &Path) -> Result<()> {
    use std::collections::HashMap;

    use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref};

    let svg_name = Name(b"S1");
    let conversion = svg2pdf::ConversionOptions::default();

    // Verovio draws noteheads, stems and beams as paths, but sets everything with
    // letters in it — fingering digits, tempo marks, titles — as real text. Those
    // need a font to become outlines, so the system fonts are loaded and a serif
    // default is named to match what the engraver asked for.
    let mut options = svg2pdf::usvg::Options {
        font_family: "Times New Roman".to_string(),
        ..Default::default()
    };
    options.fontdb_mut().load_system_fonts();

    let mut alloc = Ref::new(1);
    let catalog_id = alloc.bump();
    let page_tree_id = alloc.bump();

    let mut pdf = Pdf::new();
    let mut page_ids = Vec::with_capacity(pages.len());
    let mut deferred = Vec::with_capacity(pages.len());

    for (index, svg) in pages.iter().enumerate() {
        let tree = svg2pdf::usvg::Tree::from_str(svg, &options)
            .with_context(|| format!("parsing engraved page {}", index + 1))?;
        let size = tree.size();

        let (chunk, svg_id) = svg2pdf::to_chunk(&tree, conversion)
            .map_err(|e| anyhow!("converting page {} to PDF: {e}", index + 1))?;

        // Every page's chunk numbers its objects from one, so they have to be
        // renumbered into a single shared space before they can share a document.
        let mut map: HashMap<Ref, Ref> = HashMap::new();
        let chunk = chunk.renumber(|old| *map.entry(old).or_insert_with(|| alloc.bump()));
        let svg_id = *map
            .get(&svg_id)
            .ok_or_else(|| anyhow!("page {} lost its root object", index + 1))?;

        let page_id = alloc.bump();
        let content_id = alloc.bump();
        page_ids.push(page_id);
        deferred.push((chunk, svg_id, page_id, content_id, size));
    }

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.pages(page_tree_id)
        .kids(page_ids.iter().copied())
        .count(page_ids.len() as i32);

    for (chunk, svg_id, page_id, content_id, size) in deferred {
        // usvg works in CSS pixels at 96 dpi; PDF works in points at 72.
        let width = size.width() * 72.0 / 96.0;
        let height = size.height() * 72.0 / 96.0;

        let mut page = pdf.page(page_id);
        page.media_box(Rect::new(0.0, 0.0, width, height));
        page.parent(page_tree_id);
        page.contents(content_id);
        let mut resources = page.resources();
        resources.x_objects().pair(svg_name, svg_id);
        resources.finish();
        page.finish();

        // The XObject is one point square, so it is scaled up to fill the page.
        let mut content = Content::new();
        content
            .transform([width, 0.0, 0.0, height, 0.0, 0.0])
            .x_object(svg_name);
        pdf.stream(content_id, &content.finish());
        pdf.extend(&chunk);
    }

    std::fs::write(output, pdf.finish())
        .with_context(|| format!("writing {}", output.display()))?;
    Ok(())
}

/// Rasterise the engraved SVG pages to PNG.
///
/// Done here rather than through Verovio's own PNG output because that path parses
/// the SVG with no fonts available, and Verovio sets everything with letters in it —
/// fingering digits included — as real text. Without fonts those simply vanish, and
/// a score with no finger numbers on it is not the thing being asked for.
fn write_pngs(pages: &[String], output: &Path) -> Result<Vec<PathBuf>> {
    let mut options = resvg::usvg::Options {
        font_family: "Times New Roman".to_string(),
        ..Default::default()
    };
    options.fontdb_mut().load_system_fonts();

    let paths = page_paths(output, pages.len(), PageFormat::Png);
    for (index, (svg, path)) in pages.iter().zip(&paths).enumerate() {
        let tree = resvg::usvg::Tree::from_str(svg, &options)
            .with_context(|| format!("parsing engraved page {}", index + 1))?;
        let size = tree.size().to_int_size().scale_by(PNG_SCALE).ok_or_else(|| {
            anyhow!("page {} has no usable size", index + 1)
        })?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
            .ok_or_else(|| anyhow!("could not allocate a {size:?} image"))?;
        pixmap.fill(resvg::tiny_skia::Color::WHITE);
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(PNG_SCALE, PNG_SCALE),
            &mut pixmap.as_mut(),
        );
        pixmap
            .save_png(path)
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(paths)
}

/// How much to enlarge a page when rasterising it, so the result is legible on
/// screen rather than merely correct.
const PNG_SCALE: f32 = 2.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_come_from_extensions() {
        assert_eq!(PageFormat::from_path(Path::new("a.pdf")), Some(PageFormat::Pdf));
        assert_eq!(PageFormat::from_path(Path::new("a.SVG")), Some(PageFormat::Svg));
        assert_eq!(PageFormat::from_path(Path::new("a.png")), Some(PageFormat::Png));
        assert_eq!(PageFormat::from_path(Path::new("a.txt")), None);
        assert_eq!(PageFormat::from_path(Path::new("a")), None);
    }

    #[test]
    fn a_single_page_keeps_the_name_it_was_given() {
        let paths = page_paths(Path::new("out/score.svg"), 1, PageFormat::Svg);
        assert_eq!(paths, vec![PathBuf::from("out/score.svg")]);
    }

    #[test]
    fn extra_pages_are_numbered() {
        let paths = page_paths(Path::new("out/score.svg"), 3, PageFormat::Svg);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("out/score.svg"),
                PathBuf::from("out/score-2.svg"),
                PathBuf::from("out/score-3.svg"),
            ]
        );
    }
}
