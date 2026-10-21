//! A minimal PNG writer (stored deflate blocks), for screenshots without an image library.

use std::io::Write;
use std::path::Path;

fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    for n in 0..256u32 {
        let mut c = n;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb88320 ^ (c >> 1) } else { c >> 1 };
        }
        t[n as usize] = c;
    }
    t
}

fn crc(t: &[u32; 256], data: &[u8]) -> u32 {
    let mut c = 0xffffffffu32;
    for b in data {
        c = t[((c ^ *b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xffffffff
}

fn chunk(out: &mut Vec<u8>, t: &[u32; 256], kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc(t, &body).to_be_bytes());
}

/// Write an RGBA image (premultiplied colours are written as they are).
pub fn write_rgba(path: &Path, w: usize, h: usize, pixels: &[egui::Color32]) -> std::io::Result<()> {
    let t = crc_table();
    let mut raw = Vec::with_capacity(h * (1 + 4 * w));
    for y in 0..h {
        raw.push(0u8);
        for p in &pixels[y * w..(y + 1) * w] {
            let [r, g, b, a] = p.to_srgba_unmultiplied();
            raw.extend_from_slice(&[r, g, b, a]);
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    while let Some(c) = chunks.next() {
        z.push(if chunks.peek().is_none() { 1 } else { 0 });
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for x in &raw {
        a = (a + *x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = vec![];
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, &t, b"IHDR", &ihdr);
    chunk(&mut out, &t, b"IDAT", &z);
    chunk(&mut out, &t, b"IEND", &[]);
    std::fs::File::create(path)?.write_all(&out)
}
