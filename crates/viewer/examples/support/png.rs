//! Minimal RGBA8 PNG writer for device-free viewer captures (stored zlib blocks).
pub fn rgba(w: u32, h: u32, pixels: &[u8]) -> Vec<u8> {
    assert_eq!(pixels.len(), (w * h * 4) as usize);
    let mut raw = Vec::new();
    for row in pixels.chunks_exact(w as usize * 4) { raw.push(0); raw.extend_from_slice(row); }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<_> = raw.chunks(65535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        z.extend_from_slice(&(b.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for x in raw { a = (a + x as u32) % 65521; b = (b + a) % 65521; }
    z.extend_from_slice(&(b << 16 | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |tag: &[u8], bytes: &[u8]| {
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        let mut data = tag.to_vec(); data.extend_from_slice(bytes);
        out.extend_from_slice(&data);
        let mut crc = !0u32;
        for byte in data {
            crc ^= byte as u32;
            for _ in 0..8 { crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 }; }
        }
        out.extend_from_slice(&(!crc).to_be_bytes());
    };
    let mut header = w.to_be_bytes().to_vec(); header.extend_from_slice(&h.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &header); chunk(b"IDAT", &z); chunk(b"IEND", &[]);
    out
}
