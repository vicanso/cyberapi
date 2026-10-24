//! What a body's bytes are, beyond what its content type says (F7.1,
//! F7.4): an image or a PDF told by its first bytes when the type names
//! none, an image's format and size, and bytes shown as hex for a body that
//! is not text.

/// The most of a body the hex view shows: 4096 lines.
pub const HEX_LIMIT: usize = 64 * 1024;
/// The widest or tallest an image's header is believed: past it, the header
/// is damaged rather than the image that large.
const IMAGE_SIDE_LIMIT: u32 = 100_000;

/// An image's format: by its first bytes, or, for one they do not tell
/// (an SVG is text), by its content type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageType {
    Png,
    Jpeg,
    Gif,
    Webp,
    Svg,
    Bmp,
    Tiff,
    Ico,
    Pnm,
    Avif,
    Heic,
    /// An image type this app knows nothing of.
    Other,
}

impl ImageType {
    /// Its name, as people write it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Gif => "GIF",
            Self::Webp => "WebP",
            Self::Svg => "SVG",
            Self::Bmp => "BMP",
            Self::Tiff => "TIFF",
            Self::Ico => "ICO",
            Self::Pnm => "PNM",
            Self::Avif => "AVIF",
            Self::Heic => "HEIC",
            Self::Other => "",
        }
    }

    /// The extension a file of it is saved with.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Svg => "svg",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
            Self::Ico => "ico",
            Self::Pnm => "pnm",
            Self::Avif => "avif",
            Self::Heic => "heic",
            Self::Other => "bin",
        }
    }

    /// Its media type, for a body whose response named none.
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Svg => "image/svg+xml",
            Self::Bmp => "image/bmp",
            Self::Tiff => "image/tiff",
            Self::Ico => "image/x-icon",
            Self::Pnm => "image/x-portable-anymap",
            Self::Avif => "image/avif",
            Self::Heic => "image/heic",
            Self::Other => "application/octet-stream",
        }
    }

    /// The format a content type names, when it names one this app knows.
    pub fn declared(content_type: &str) -> Option<Self> {
        let lower = content_type.to_ascii_lowercase();
        let subtype = lower.split(';').next()?.split_once('/')?.1.trim().to_string();
        let known = [
            ("png", Self::Png),
            ("jpeg", Self::Jpeg),
            ("jpg", Self::Jpeg),
            ("pjpeg", Self::Jpeg),
            ("gif", Self::Gif),
            ("webp", Self::Webp),
            ("svg+xml", Self::Svg),
            ("svg", Self::Svg),
            ("bmp", Self::Bmp),
            ("x-ms-bmp", Self::Bmp),
            ("tiff", Self::Tiff),
            ("x-icon", Self::Ico),
            ("vnd.microsoft.icon", Self::Ico),
            ("avif", Self::Avif),
            ("heic", Self::Heic),
            ("heif", Self::Heic),
        ];
        known
            .into_iter()
            .find(|(name, _)| *name == subtype)
            .map(|(_, kind)| kind)
            .or_else(|| subtype.starts_with("x-portable-").then_some(Self::Pnm))
    }
}

/// What a body's first bytes say it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sniffed {
    Image(ImageType),
    Pdf,
}

/// The image or PDF `bytes` start as, by the signature every file of the
/// format starts with. An SVG is text, and not told this way.
pub fn sniff(bytes: &[u8]) -> Option<Sniffed> {
    let image = |kind| Some(Sniffed::Image(kind));
    match bytes {
        [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, ..] => image(ImageType::Png),
        [0xff, 0xd8, 0xff, ..] => image(ImageType::Jpeg),
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => image(ImageType::Gif),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => image(ImageType::Webp),
        [b'B', b'M', _, _, _, _, 0, 0, 0, 0, ..] => image(ImageType::Bmp),
        [b'I', b'I', 0x2a, 0x00, ..] | [b'M', b'M', 0x00, 0x2a, ..] => image(ImageType::Tiff),
        [0, 0, 1, 0, count_low, count_high, ..] if (*count_low, *count_high) != (0, 0) => image(ImageType::Ico),
        [b'P', b'1'..=b'7', b' ' | b'\t' | b'\n' | b'\r', ..] => image(ImageType::Pnm),
        [_, _, _, _, b'f', b't', b'y', b'p', brand @ ..] => match brand.get(..4) {
            Some(b"avif" | b"avis") => image(ImageType::Avif),
            Some(b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"msf1") => image(ImageType::Heic),
            _ => None,
        },
        [b'%', b'P', b'D', b'F', b'-', ..] => Some(Sniffed::Pdf),
        _ => None,
    }
}

/// An image's width and height in its own pixels, when its header says, and
/// says something an image could be.
pub fn image_size(bytes: &[u8], kind: ImageType) -> Option<(u32, u32)> {
    let (width, height) = match kind {
        ImageType::Svg => svg_size(std::str::from_utf8(bytes).ok()?)?,
        ImageType::Other => return None,
        _ => {
            let size = imagesize::blob_size(bytes).ok()?;
            (u32::try_from(size.width).ok()?, u32::try_from(size.height).ok()?)
        }
    };
    let believable = |side| (1..=IMAGE_SIDE_LIMIT).contains(&side);
    (believable(width) && believable(height)).then_some((width, height))
}

/// An SVG's size as its root element gives it: `width` and `height` in
/// pixels, or else its `viewBox`. A size in other units (`em`, `%`) is not
/// one.
fn svg_size(text: &str) -> Option<(u32, u32)> {
    let start = text.find("<svg")?;
    let tag = &text[start..];
    let tag = &tag[..tag.find('>')?];
    let length = |name| {
        let value = attribute(tag, name)?;
        let number = value.trim().trim_end_matches("px").trim();
        let number: f64 = number.parse().ok()?;
        (number.is_finite() && number > 0.).then(|| number.round() as u32)
    };
    if let (Some(width), Some(height)) = (length("width"), length("height")) {
        return Some((width, height));
    }
    let view_box: Vec<f64> = attribute(tag, "viewBox")?
        .split(|ch: char| ch.is_whitespace() || ch == ',')
        .filter(|part| !part.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    match view_box[..] {
        [_, _, width, height] if width > 0. && height > 0. => Some((width.round() as u32, height.round() as u32)),
        _ => None,
    }
}

/// An attribute's value in an element's opening tag: `name="…"` or
/// `name='…'`, the name whole (`width`, not `stroke-width`).
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    while let Some(at) = rest.find(name) {
        let before = rest[..at].chars().next_back();
        let after = rest[at + name.len()..].trim_start();
        rest = &rest[at + name.len()..];
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let Some(value) = after.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let quote = value.chars().next().filter(|ch| *ch == '"' || *ch == '\'')?;
        let value = &value[1..];
        return Some(&value[..value.find(quote)?]);
    }
    None
}

/// `bytes` as `hexdump -C` writes them: the offset, sixteen bytes in hex,
/// and those that are printable ASCII as themselves.
pub fn hex_dump(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(16) * 79);
    for (line, chunk) in bytes.chunks(16).enumerate() {
        out.push_str(&format!("{:08x} ", line * 16));
        for column in 0..16 {
            if column == 8 {
                out.push(' ');
            }
            match chunk.get(column) {
                Some(byte) => out.push_str(&format!(" {byte:02x}")),
                None => out.push_str("   "),
            }
        }
        out.push_str("  |");
        out.extend(chunk.iter().map(|&byte| {
            if byte.is_ascii_graphic() || byte == b' ' {
                byte as char
            } else {
                '.'
            }
        }));
        out.push_str("|\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_HEADER: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00,
        0x02, 0x80, 0x00, 0x00, 0x01, 0xe0, 0x08, 0x06, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn the_first_bytes_tell_images_and_pdf() {
        let image = |kind| Some(Sniffed::Image(kind));
        assert_eq!(sniff(PNG_HEADER), image(ImageType::Png));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0, 0, 0x10]), image(ImageType::Jpeg));
        assert_eq!(sniff(b"GIF89a\x01\x00"), image(ImageType::Gif));
        assert_eq!(sniff(b"RIFF\x24\x00\x00\x00WEBPVP8 "), image(ImageType::Webp));
        assert_eq!(sniff(b"BM\x3a\x00\x00\x00\x00\x00\x00\x00\x36"), image(ImageType::Bmp));
        assert_eq!(sniff(b"II*\x00\x08\x00"), image(ImageType::Tiff));
        assert_eq!(sniff(&[0, 0, 1, 0, 1, 0, 16, 16]), image(ImageType::Ico));
        assert_eq!(sniff(b"P6\n2 2\n255\n"), image(ImageType::Pnm));
        assert_eq!(sniff(b"\x00\x00\x00\x1cftypavif\x00\x00"), image(ImageType::Avif));
        assert_eq!(sniff(b"\x00\x00\x00\x18ftypheic"), image(ImageType::Heic));
        assert_eq!(sniff(b"%PDF-1.7\n"), Some(Sniffed::Pdf));
        assert_eq!(sniff(b"\x00\x00\x00\x18ftypmp42"), None, "a video");
        assert_eq!(sniff(b"PK\x03\x04"), None, "a zip");
        assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), None, "text");
        assert_eq!(sniff(b"BM"), None, "too short to be one");
        assert_eq!(sniff(&[0, 0, 1, 0, 0, 0]), None, "an icon of no images");
    }

    #[test]
    fn a_content_type_names_the_formats_it_knows() {
        assert_eq!(ImageType::declared("image/png"), Some(ImageType::Png));
        assert_eq!(ImageType::declared("IMAGE/JPEG; charset=binary"), Some(ImageType::Jpeg));
        assert_eq!(ImageType::declared("image/svg+xml"), Some(ImageType::Svg));
        assert_eq!(ImageType::declared("image/vnd.microsoft.icon"), Some(ImageType::Ico));
        assert_eq!(ImageType::declared("image/x-portable-pixmap"), Some(ImageType::Pnm));
        assert_eq!(ImageType::declared("image/jxl"), None);
        assert_eq!(ImageType::declared(""), None);
    }

    #[test]
    fn an_images_size_is_read_from_its_header() {
        assert_eq!(image_size(PNG_HEADER, ImageType::Png), Some((640, 480)));
        assert_eq!(
            image_size(
                b"GIF89a\x20\x00\x10\x00\xf7\x00\x00\x00\x00\x00\x00\x00",
                ImageType::Gif
            ),
            Some((32, 16))
        );
        assert_eq!(image_size(&PNG_HEADER[..12], ImageType::Png), None, "cut short");
        let mut damaged = PNG_HEADER.to_vec();
        damaged[16..20].copy_from_slice(&[0x08, 0x09, 0x0a, 0x0b]);
        assert_eq!(image_size(&damaged, ImageType::Png), None, "134 million pixels wide");
        assert_eq!(image_size(b"", ImageType::Other), None);
    }

    #[test]
    fn an_svgs_size_is_its_root_elements() {
        let size = |text: &str| image_size(text.as_bytes(), ImageType::Svg);
        assert_eq!(size(r#"<svg width="120" height="80"><rect/></svg>"#), Some((120, 80)));
        assert_eq!(
            size(r#"<?xml version="1.0"?><svg height='24px' width='48px'>"#),
            Some((48, 24))
        );
        assert_eq!(size(r#"<svg viewBox="0 0 24 12" stroke-width="2">"#), Some((24, 12)));
        assert_eq!(
            size(r#"<svg width="100%" height="100%" viewBox="0,0,300,150">"#),
            Some((300, 150)),
            "not pixels: the view box"
        );
        assert_eq!(size(r#"<svg stroke-width="3" width="5" height="6">"#), Some((5, 6)));
        assert_eq!(size("<svg>"), None);
        assert_eq!(size("<html>"), None);
    }

    #[test]
    fn bytes_are_shown_as_hexdump_shows_them() {
        assert_eq!(
            hex_dump(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x01"),
            "00000000  89 50 4e 47 0d 0a 1a 0a  00 00 00 0d 49 48 44 52  |.PNG........IHDR|\n\
             00000010  00 01                                             |..|\n"
        );
        assert_eq!(hex_dump(b""), "");
        assert_eq!(
            hex_dump(b"a b~"),
            format!("00000000  61 20 62 7e{}  |a b~|\n", " ".repeat(37))
        );
    }
}
