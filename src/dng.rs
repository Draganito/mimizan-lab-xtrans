//! The developed picture as a linear DNG.
//!
//! "Linear" is the DNG's word for a raw file that is no mosaic any more:
//! three channels per pixel in the camera's own colours, not balanced and
//! without a tone curve, with the camera's matrix and balance beside
//! them. A raw developer opens it like the RAF and leaves out one step.

use crate::convert::{Developed, WHITE};
use rawler::decoders::{Decoder, RawDecodeParams};
use rawler::dng::writer::DngWriter;
use rawler::dng::{CropMode, DngCompression, DngPhotometricConversion, DNG_VERSION_V1_4};
use rawler::rawimage::{BlackLevel, RawPhotometricInterpretation, WhiteLevel};
use rawler::rawsource::RawSource;
use rawler::tags::{DngTag, ExifTag, TiffCommonTag};
use rawler::{RawImage, RawImageData};
use std::io::{Seek, Write};

/// Quality of the preview a file browser shows, of 1.
const PREVIEW_QUALITY: f32 = 0.75;

/// Write `dev`, the picture of `img`, to `out`. The camera, the exposure
/// data, the balance, the colour matrices and the preview are the RAF's.
pub fn write<W: Write + Seek>(
    out: W,
    source: &RawSource,
    decoder: &dyn Decoder,
    mut img: RawImage,
    dev: Developed,
    compressed: bool,
    software: &str,
) -> Result<(), String> {
    let params = RawDecodeParams::default();
    let metadata = decoder.raw_metadata(source, &params).map_err(|e| format!("metadata: {e}"))?;

    // The RAF's description, with the picture in place of the mosaic.
    img.width = dev.frame.width;
    img.height = dev.frame.height;
    img.cpp = 3;
    img.bps = 16;
    img.photometric = RawPhotometricInterpretation::LinearRaw;
    img.whitelevel = WhiteLevel::new(vec![u32::from(WHITE); 3]);
    img.blacklevel = BlackLevel::new(&[0_u16; 3], 1, 1, 3);
    img.active_area = None;
    img.crop_area = None;
    img.blackareas.clear();
    if !dev.balance_from_file {
        img.wb_coeffs = [dev.balance[0] as f32, dev.balance[1] as f32, dev.balance[2] as f32, f32::NAN];
    }
    img.data = RawImageData::Integer(dev.data);

    let failed = |what: &'static str| move |e| format!("{what}: {e}");
    // The camera's own JPEG is the preview and the thumbnail. A DNG's
    // first directory is its thumbnail; a RAF without a JPEG gets the
    // picture there instead.
    let jpeg = decoder.preview_image(source, &params).ok().flatten();
    let mut dng = DngWriter::new(out, DNG_VERSION_V1_4).map_err(failed("DNG"))?;
    let mut raw = if jpeg.is_some() { dng.subframe(0) } else { dng.subframe_on_root(0) };
    let compression = if compressed { DngCompression::Lossless } else { DngCompression::Uncompressed };
    raw.raw_image(&img, CropMode::None, compression, DngPhotometricConversion::Original, 1)
        .map_err(failed("picture"))?;
    raw.ifd_mut().add_tag(DngTag::DefaultCropOrigin, [0_u16, 0]);
    raw.ifd_mut().add_tag(DngTag::DefaultCropSize, [img.width as u16, img.height as u16]);
    raw.finalize().map_err(failed("picture"))?;
    if let Some(jpeg) = &jpeg {
        let mut preview = dng.subframe(1);
        preview.preview(jpeg, PREVIEW_QUALITY).map_err(failed("preview"))?;
        preview.finalize().map_err(failed("preview"))?;
        dng.thumbnail(jpeg).map_err(failed("thumbnail"))?;
    }

    dng.load_base_tags(&img).map_err(failed("camera"))?;
    dng.load_metadata(&metadata).map_err(failed("metadata"))?;
    if !dng.root_ifd().contains(ExifTag::Orientation) {
        dng.root_ifd_mut().add_tag(ExifTag::Orientation, img.orientation.to_u16());
    }
    if let Ok(Some(xpacket)) = decoder.xpacket(source, &params) {
        dng.xpacket(&xpacket).map_err(failed("XMP"))?;
    }
    dng.root_ifd_mut().add_tag(TiffCommonTag::Software, software);
    dng.close().map_err(failed("DNG"))
}
