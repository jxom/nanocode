use std::sync::OnceLock;

use crate::decode::{DecodeError, Decoded, Decoder, Format, Header, Limits};
use crate::pixels::{Image, Pixels};

static CODEC: OnceLock<&'static dyn Codec> = OnceLock::new();

pub trait Codec: Send + Sync {
    fn decode(&self, decoder: Decoder<'_>) -> Result<Decoded, DecodeError>;
    fn encode(&self, encoding: Encoding<'_>) -> Result<Vec<u8>, String>;
    fn inspect(&self, bytes: &[u8], format: Format, limits: Limits) -> Result<Header, DecodeError>;
    fn resize(&self, image: &Image, width: u32, height: u32, filter: Filter) -> Image;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Lanczos3,
    Thumbnail,
}

#[derive(Clone, Copy)]
pub struct Encoding<'a> {
    pub exif: Option<&'a [u8]>,
    pub format: Format,
    pub icc_profile: Option<&'a [u8]>,
    pub image: &'a Image,
    pub quality: u8,
}

pub struct Builtin;

impl Codec for Builtin {
    fn decode(&self, decoder: Decoder<'_>) -> Result<Decoded, DecodeError> {
        decoder.decode_builtin()
    }

    fn inspect(&self, bytes: &[u8], format: Format, limits: Limits) -> Result<Header, DecodeError> {
        crate::decode::parse(bytes, format, limits).map(|(header, _)| header)
    }

    fn resize(&self, image: &Image, width: u32, height: u32, filter: Filter) -> Image {
        match filter {
            Filter::Lanczos3 => crate::resample::resample(image, width, height),
            Filter::Thumbnail => crate::thumbnail::sample(image, width, height),
        }
    }

    fn encode(&self, encoding: Encoding<'_>) -> Result<Vec<u8>, String> {
        let Encoding {
            exif,
            format,
            icc_profile,
            image,
            quality,
        } = encoding;
        match format {
            Format::Png => {
                use nanocode_png::{Color, Pixels as Samples};
                let (color, pixels) = match image.pixels() {
                    Pixels::Luma8(samples) => (Color::L8, Samples::Eight(samples)),
                    Pixels::LumaAlpha8(samples) => (Color::La8, Samples::Eight(samples)),
                    Pixels::Rgb8(samples) => (Color::Rgb8, Samples::Eight(samples)),
                    Pixels::Rgba8(samples) => (Color::Rgba8, Samples::Eight(samples)),
                    Pixels::Luma16(samples) => (Color::L16, Samples::Sixteen(samples)),
                    Pixels::LumaAlpha16(samples) => (Color::La16, Samples::Sixteen(samples)),
                    Pixels::Rgb16(samples) => (Color::Rgb16, Samples::Sixteen(samples)),
                    Pixels::Rgba16(samples) => (Color::Rgba16, Samples::Sixteen(samples)),
                };
                Ok(nanocode_png::encode(&nanocode_png::Encoding {
                    width: image.width(),
                    height: image.height(),
                    color,
                    pixels,
                    icc_profile: icc_profile.filter(|data| !data.is_empty()),
                    exif: exif.filter(|data| !data.is_empty()),
                }))
            }
            Format::Jpeg => {
                let mut bytes = nanocode_jpeg::encode(
                    &image.to_rgb8(),
                    image.width(),
                    image.height(),
                    nanocode_jpeg::Color::Rgb,
                    quality,
                    icc_profile,
                )
                .map_err(|error| format!("unable to encode image as Jpeg: {error}"))?;
                if let Some(exif) = exif.filter(|data| !data.is_empty()) {
                    insert_jpeg_exif(&mut bytes, exif)?;
                }
                Ok(bytes)
            }
            Format::WebP => {
                let (pixels, layout) = image.rgb_or_rgba8();
                nanocode_webp::encode(
                    pixels.as_ref(),
                    image.width(),
                    image.height(),
                    layout == crate::RgbLayout::Rgba,
                    icc_profile,
                    exif,
                )
                .map_err(|error| format!("unable to encode image as WebP: {error}"))
            }
            other => Err(format!("unable to encode image as {other:?}: unsupported")),
        }
    }
}

pub fn install(codec: &'static dyn Codec) {
    let _ = CODEC.set(codec);
}

#[cfg(target_family = "wasm")]
pub(crate) fn installed() -> Option<&'static dyn Codec> {
    CODEC.get().copied()
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn installed() -> Option<&'static dyn Codec> {
    Some(CODEC.get().copied().unwrap_or(&Builtin))
}

pub(crate) fn resize(image: &Image, width: u32, height: u32, filter: Filter) -> Image {
    installed().map_or_else(
        || image.clone(),
        |codec| codec.resize(image, width, height, filter),
    )
}

pub fn encode(encoding: Encoding<'_>) -> Result<Vec<u8>, String> {
    installed()
        .ok_or_else(|| {
            format!(
                "unable to encode image as {:?}: no codec is installed",
                encoding.format
            )
        })?
        .encode(encoding)
}

fn insert_jpeg_exif(bytes: &mut Vec<u8>, exif: &[u8]) -> Result<(), String> {
    let length = exif
        .len()
        .checked_add(2 + b"Exif\0\0".len())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or_else(|| {
            "unable to encode image as Jpeg: Exif metadata exceeds the JPEG APP1 segment length"
                .to_owned()
        })?;
    let mut segment = Vec::with_capacity(2 + usize::from(length));
    segment.extend_from_slice(&[0xff, 0xe1]);
    segment.extend_from_slice(&length.to_be_bytes());
    segment.extend_from_slice(b"Exif\0\0");
    segment.extend_from_slice(exif);
    bytes.splice(2..2, segment);
    Ok(())
}
