fn main() {
    let a: Vec<String> = std::env::args().collect();
    let d = std::fs::read(&a[1]).unwrap();
    let t = libsms_assets::texture::read_bti(&d, a[1].clone()).unwrap();
    let mut out = Vec::new();
    out.extend_from_slice(&t.image.width.to_le_bytes());
    out.extend_from_slice(&t.image.height.to_le_bytes());
    out.extend_from_slice(&t.image.rgba);
    std::fs::write(&a[2], out).unwrap();
}
