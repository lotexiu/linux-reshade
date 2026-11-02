use super::*;

fn encode(width: u32, height: u32, color: png::ColorType, depth: png::BitDepth, data: &[u8], palette: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(depth);
        if let Some(p) = palette {
            encoder.set_palette(p.to_vec());
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(data).unwrap();
    }
    out
}

#[test]
fn rgba_passes_through() {
    let px = [1, 2, 3, 4, 5, 6, 7, 8];
    let png = encode(2, 1, png::ColorType::Rgba, png::BitDepth::Eight, &px, None);
    assert_eq!(decode("a.png", &png).unwrap(), Rgba8 { width: 2, height: 1, pixels: px.to_vec() });
}

#[test]
fn rgb_gets_an_opaque_alpha() {
    let png = encode(1, 2, png::ColorType::Rgb, png::BitDepth::Eight, &[10, 20, 30, 40, 50, 60], None);
    assert_eq!(decode("a.png", &png).unwrap().pixels, [10, 20, 30, 255, 40, 50, 60, 255]);
}

#[test]
fn grayscale_and_gray_alpha_spread_to_all_channels() {
    let g = encode(2, 1, png::ColorType::Grayscale, png::BitDepth::Eight, &[7, 200], None);
    assert_eq!(decode("g.png", &g).unwrap().pixels, [7, 7, 7, 255, 200, 200, 200, 255]);
    let ga = encode(1, 1, png::ColorType::GrayscaleAlpha, png::BitDepth::Eight, &[9, 128], None);
    assert_eq!(decode("ga.png", &ga).unwrap().pixels, [9, 9, 9, 128]);
}

#[test]
fn a_palette_is_expanded() {
    let png = encode(2, 1, png::ColorType::Indexed, png::BitDepth::Eight, &[1, 0], Some(&[255, 0, 0, 0, 0, 255]));
    assert_eq!(decode("p.png", &png).unwrap().pixels, [0, 0, 255, 255, 255, 0, 0, 255]);
}

#[test]
fn sixteen_bits_become_eight() {
    // 0xFFFF e 0x8000 por canal (big endian).
    let png = encode(1, 1, png::ColorType::Rgba, png::BitDepth::Sixteen, &[0xFF, 0xFF, 0x80, 0x00, 0x00, 0x00, 0xFF, 0xFF], None);
    let d = decode("s.png", &png).unwrap();
    assert_eq!(d.pixels[0], 255);
    assert!(d.pixels[1].abs_diff(128) <= 1);
    assert_eq!((d.pixels[2], d.pixels[3]), (0, 255));
}

#[test]
fn other_formats_and_garbage_are_refused_with_a_reason() {
    assert!(matches!(decode("lut.dds", b"DDS \x7c\0\0\0"), Err(Error::Unsupported(m)) if m.contains("dds")));
    assert!(matches!(decode("x", b""), Err(Error::Unsupported(_))));
    let mut png = encode(1, 1, png::ColorType::Rgba, png::BitDepth::Eight, &[1, 2, 3, 4], None);
    png.truncate(png.len() - 20);
    assert!(matches!(decode("cortado.png", &png), Err(Error::Decode(_))));
    assert!(matches!(decode("lixo.png", b"\x89PNG\r\n\x1a\nlixo lixo lixo"), Err(Error::Decode(_))));
}

#[test]
fn jpeg_is_decoded_to_rgba() {
    let img = decode("t.jpg", include_bytes!("test_red.jpg")).unwrap();
    assert_eq!((img.width, img.height), (8, 8));
    let p = &img.pixels[..4];
    assert!((i32::from(p[0]) - 200).abs() < 6 && (i32::from(p[1]) - 40).abs() < 6 && (i32::from(p[2]) - 40).abs() < 6 && p[3] == 255, "{p:?}");
}

#[test]
fn truncated_jpeg_is_an_error_not_a_panic() {
    let bytes = include_bytes!("test_red.jpg");
    assert!(decode("t.jpg", &bytes[..40]).is_err());
}

#[test]
fn encoded_png_decodes_back_to_the_same_pixels() {
    let rgba: Vec<u8> = (0..4u8 * 3).flat_map(|n| [n * 20, 255 - n * 10, n, 77]).collect();
    let png = encode_png(4, 3, &rgba).unwrap();
    let back = decode("captura.png", &png).unwrap();
    assert_eq!((back.width, back.height), (4, 3));
    // O alfa sai (a captura é opaca): volta 255.
    let want: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
    assert_eq!(back.pixels, want);
    assert!(encode_png(2, 2, &[0; 3]).is_err());
}
