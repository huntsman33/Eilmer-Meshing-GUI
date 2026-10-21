//! A small CPU rasterizer for egui output, for headless screenshots in tests and documentation
//! (no window, no GPU).

use std::collections::HashMap;

use egui::epaint::{ClippedPrimitive, ImageData, Primitive, TextureId};
use egui::{Color32, FullOutput};

#[derive(Default)]
pub struct SoftRenderer {
    textures: HashMap<TextureId, (usize, usize, Vec<[f32; 4]>)>,
}


fn to_f(c: Color32) -> [f32; 4] {
    [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, c.a() as f32 / 255.0]
}

impl SoftRenderer {
    /// Keep the textures from a frame's output (call for every frame).
    pub fn update_textures(&mut self, out: &FullOutput) {
        for (id, delta) in &out.textures_delta.set {
            let (w, h, pixels): (usize, usize, Vec<[f32; 4]>) = match &delta.image {
                ImageData::Color(img) => (img.width(), img.height(), img.pixels.iter().map(|c| to_f(*c)).collect()),
                ImageData::Font(f) => (f.width(), f.height(), f.srgba_pixels(None).map(to_f).collect()),
            };
            match delta.pos {
                None => {
                    self.textures.insert(*id, (w, h, pixels));
                }
                Some([x0, y0]) => {
                    if let Some((tw, _, t)) = self.textures.get_mut(id) {
                        for y in 0..h {
                            for x in 0..w {
                                t[(y0 + y) * *tw + x0 + x] = pixels[y * w + x];
                            }
                        }
                    }
                }
            }
        }
        for id in &out.textures_delta.free {
            self.textures.remove(id);
        }
    }

    fn sample(&self, id: TextureId, u: f32, v: f32) -> [f32; 4] {
        let Some((w, h, t)) = self.textures.get(&id) else { return [1.0; 4] };
        let x = ((u * *w as f32) as isize).clamp(0, *w as isize - 1) as usize;
        let y = ((v * *h as f32) as isize).clamp(0, *h as isize - 1) as usize;
        t[y * w + x]
    }

    /// Rasterize tessellated primitives into a width x height image (premultiplied blending).
    pub fn render(&self, prims: &[ClippedPrimitive], width: usize, height: usize, background: Color32) -> Vec<Color32> {
        let bg = to_f(background);
        let mut img = vec![bg; width * height];
        for cp in prims {
            let Primitive::Mesh(mesh) = &cp.primitive else { continue };
            let clip = cp.clip_rect;
            for tri in mesh.indices.chunks(3) {
                let [a, b, c] = [&mesh.vertices[tri[0] as usize], &mesh.vertices[tri[1] as usize], &mesh.vertices[tri[2] as usize]];
                let x0 = a.pos.x.min(b.pos.x).min(c.pos.x).max(clip.min.x).max(0.0).floor() as isize;
                let x1 = a.pos.x.max(b.pos.x).max(c.pos.x).min(clip.max.x).min(width as f32).ceil() as isize;
                let y0 = a.pos.y.min(b.pos.y).min(c.pos.y).max(clip.min.y).max(0.0).floor() as isize;
                let y1 = a.pos.y.max(b.pos.y).max(c.pos.y).min(clip.max.y).min(height as f32).ceil() as isize;
                let area = (b.pos.x - a.pos.x) * (c.pos.y - a.pos.y) - (b.pos.y - a.pos.y) * (c.pos.x - a.pos.x);
                if area.abs() < 1e-12 {
                    continue;
                }
                let (ca, cb, cc) = (to_f(a.color), to_f(b.color), to_f(c.color));
                for y in y0.max(0)..y1 {
                    for x in x0.max(0)..x1 {
                        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                        let w0 = ((b.pos.x - px) * (c.pos.y - py) - (b.pos.y - py) * (c.pos.x - px)) / area;
                        let w1 = ((c.pos.x - px) * (a.pos.y - py) - (c.pos.y - py) * (a.pos.x - px)) / area;
                        let w2 = 1.0 - w0 - w1;
                        // top-left rule: a pixel on an edge shared by two triangles is drawn once
                        let inside = |w: f32, e0: egui::Pos2, e1: egui::Pos2| {
                            w > 0.0 || (w == 0.0 && ((e1.y - e0.y) * area.signum() < 0.0 || (e1.y == e0.y && (e1.x - e0.x) * area.signum() > 0.0)))
                        };
                        if !(inside(w0, b.pos, c.pos) && inside(w1, c.pos, a.pos) && inside(w2, a.pos, b.pos)) {
                            continue;
                        }
                        let u = w0 * a.uv.x + w1 * b.uv.x + w2 * c.uv.x;
                        let v = w0 * a.uv.y + w1 * b.uv.y + w2 * c.uv.y;
                        let t = self.sample(mesh.texture_id, u, v);
                        let mut s = [0.0f32; 4];
                        for k in 0..4 {
                            s[k] = (w0 * ca[k] + w1 * cb[k] + w2 * cc[k]) * t[k];
                        }
                        let d = &mut img[y as usize * width + x as usize];
                        for k in 0..4 {
                            d[k] = s[k] + d[k] * (1.0 - s[3]);
                        }
                    }
                }
            }
        }
        img.iter()
            .map(|p| {
                let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                Color32::from_rgba_premultiplied(q(p[0]), q(p[1]), q(p[2]), q(p[3]))
            })
            .collect()
    }
}
