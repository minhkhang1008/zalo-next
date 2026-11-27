//! `COLR` (v0 and v1) colour glyph renderer.
//!
//! Windows' Segoe UI Emoji, and the COLRv1 build of Noto Color Emoji, store emoji as layered
//! vector paints instead of PNG strikes, so `glyph_raster_image` finds nothing for them. This
//! walks the paint graph through `ttf_parser`'s `colr::Painter` callbacks: outlines are flattened
//! and filled with `ab_glyph_rasterizer` (the coverage rasteriser egui already ships), and every
//! paint (solid, linear, radial and sweep gradients) is evaluated per pixel on premultiplied RGBA
//! layers that are composited with the `PaintComposite` mode on pop.

use ab_glyph_rasterizer::{point, Point, Rasterizer};
use rustybuzz::ttf_parser::{self, colr, GlyphId, OutlineBuilder, RgbaColor, Transform};

/// Most colour glyphs painted for one cluster (a flag drawn as two letters needs 2, an
/// unsupported family sequence a handful).
const MAX_GLYPHS: usize = 8;

/// One glyph to paint and its pen offset along the baseline, in font units.
pub(super) struct PlacedGlyph {
    pub id: GlyphId,
    pub x: f32,
}

/// Paint `glyphs` (already shaped) into a square RGBA image whose edge is about `size` pixels per
/// em-box. Returns `None` when none of the glyphs has a `COLR` definition or nothing was drawn.
pub(super) fn render(
    face: &ttf_parser::Face<'_>,
    glyphs: &[PlacedGlyph],
    total_advance: f32,
    size: u32,
) -> Option<image::RgbaImage> {
    let colour: Vec<&PlacedGlyph> = glyphs
        .iter()
        .filter(|g| face.is_color_glyph(g.id))
        .collect();
    // A cluster from an incoming message can be an arbitrarily long joined run; past a few
    // glyphs it is not one emoji any more, so leave it to the text renderer.
    if colour.is_empty() || colour.len() > MAX_GLYPHS || glyphs.len() > 4 * MAX_GLYPHS || size == 0
    {
        return None;
    }

    let upem = f32::from(face.units_per_em().max(1));
    let ascender = f32::from(face.ascender());
    let descender = f32::from(face.descender());
    let height = if ascender - descender > 0.0 {
        ascender - descender
    } else {
        upem
    };
    let width = if total_advance > 0.0 {
        total_advance
    } else {
        upem
    };
    // One em-box (the taller of advance and line height) maps to `size` pixels; the canvas is
    // twice as large so outlines that overshoot the box are not cut before the crop below.
    let single_advance = width / colour.len() as f32;
    let scale = size as f32 / single_advance.max(height);
    // Capped so a long run of glyphs cannot allocate an unbounded canvas.
    let canvas = (size as usize * 2)
        .max((width * scale).ceil() as usize + size as usize)
        .min(size as usize * 8);
    let half = canvas as f32 / 2.0;
    let (cx, cy) = (width / 2.0, (ascender + descender) / 2.0);

    let mut painter = ColrPainter::new(face, canvas);
    let foreground = RgbaColor::new(0, 0, 0, 255);
    for glyph in colour {
        // Font units are y-up; pixels are y-down. Centre the line box on the canvas.
        let base = Transform::new(
            scale,
            0.0,
            0.0,
            -scale,
            half + (glyph.x - cx) * scale,
            half + cy * scale,
        );
        painter.reset_transform(base);
        face.paint_color_glyph(glyph.id, 0, foreground, &mut painter)?;
    }
    painter.into_image(size)
}

/// Premultiplied RGBA with components in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Px {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl Px {
    fn from_rgba(c: RgbaColor) -> Self {
        let a = f32::from(c.alpha) / 255.0;
        Px {
            r: f32::from(c.red) / 255.0 * a,
            g: f32::from(c.green) / 255.0 * a,
            b: f32::from(c.blue) / 255.0 * a,
            a,
        }
    }

    fn scale(self, k: f32) -> Self {
        Px {
            r: self.r * k,
            g: self.g * k,
            b: self.b * k,
            a: self.a * k,
        }
    }

    fn add(self, o: Self) -> Self {
        Px {
            r: self.r + o.r,
            g: self.g + o.g,
            b: self.b + o.b,
            a: self.a + o.a,
        }
    }

    fn lerp(self, o: Self, t: f32) -> Self {
        self.scale(1.0 - t).add(o.scale(t))
    }

    fn rgb(self) -> [f32; 3] {
        [self.r, self.g, self.b]
    }

    /// Apply `f` to the unpremultiplied colour channels, keeping alpha.
    fn map_unpremultiplied(self, f: fn(f32) -> f32) -> Self {
        if self.a <= 0.0 {
            return Px::default();
        }
        let a = self.a;
        Px {
            r: f(self.r / a) * a,
            g: f(self.g / a) * a,
            b: f(self.b / a) * a,
            a,
        }
    }

    fn to_linear(self) -> Self {
        self.map_unpremultiplied(srgb_to_linear)
    }

    fn to_srgb(self) -> Self {
        self.map_unpremultiplied(linear_to_srgb)
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

struct ColrPainter<'f, 'a> {
    face: &'f ttf_parser::Face<'a>,
    size: usize,
    /// Current transform stack, font units to pixels; the last entry is in effect.
    transforms: Vec<Transform>,
    /// Coverage of the outline from the latest `outline_glyph`, not yet used as a clip (COLRv0).
    outline: Option<Vec<f32>>,
    /// Cumulative clip masks (each one already intersected with the one below).
    clips: Vec<Vec<f32>>,
    /// Layer stack; the first entry is the output and is never popped.
    layers: Vec<(Vec<Px>, colr::CompositeMode)>,
}

impl<'f, 'a> ColrPainter<'f, 'a> {
    fn new(face: &'f ttf_parser::Face<'a>, size: usize) -> Self {
        ColrPainter {
            face,
            size,
            transforms: vec![Transform::default()],
            outline: None,
            clips: Vec::new(),
            layers: vec![(
                vec![Px::default(); size * size],
                colr::CompositeMode::SourceOver,
            )],
        }
    }

    fn reset_transform(&mut self, base: Transform) {
        self.transforms.clear();
        self.transforms.push(base);
        self.outline = None;
        self.clips.clear();
        self.layers.truncate(1);
    }

    fn ctm(&self) -> Transform {
        self.transforms.last().copied().unwrap_or_default()
    }

    fn rasterize(&self, draw: impl FnOnce(&mut Flattener<'_>)) -> Vec<f32> {
        let mut raster = Rasterizer::new(self.size, self.size);
        {
            let mut flattener = Flattener {
                raster: &mut raster,
                ctm: self.ctm(),
                width: self.size as f32,
                start: point(0.0, 0.0),
                last: point(0.0, 0.0),
                open: false,
            };
            draw(&mut flattener);
            flattener.close();
        }
        let mut mask = vec![0.0; self.size * self.size];
        raster.for_each_pixel(|i, c| {
            if let Some(m) = mask.get_mut(i) {
                *m = c.min(1.0);
            }
        });
        mask
    }

    fn push_mask(&mut self, mut mask: Vec<f32>) {
        if let Some(top) = self.clips.last() {
            for (m, t) in mask.iter_mut().zip(top) {
                *m *= t;
            }
        }
        self.clips.push(mask);
    }

    fn into_image(self, size: u32) -> Option<image::RgbaImage> {
        let n = self.size;
        let (pixels, _) = self.layers.into_iter().next()?;
        // Bounding box of everything drawn.
        let (mut x0, mut y0, mut x1, mut y1) = (n, n, 0usize, 0usize);
        for (i, p) in pixels.iter().enumerate() {
            if p.a > 1.0 / 255.0 {
                let (x, y) = (i % n, i / n);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        // Crop a square of at least one em around the drawing, so relative emoji sizes match the
        // font's design instead of every emoji being blown up to fill its cell.
        let edge = (size as usize).max(x1 - x0).max(y1 - y0).min(n);
        let left = ((x0 + x1) / 2).saturating_sub(edge / 2).min(n - edge);
        let top = ((y0 + y1) / 2).saturating_sub(edge / 2).min(n - edge);

        let mut img = image::RgbaImage::new(edge as u32, edge as u32);
        for (x, y, out) in img.enumerate_pixels_mut() {
            let p = pixels[(top + y as usize) * n + left + x as usize];
            let a = p.a.clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let un = |c: f32| ((c / a).clamp(0.0, 1.0) * 255.0).round() as u8;
            *out = image::Rgba([un(p.r), un(p.g), un(p.b), (a * 255.0).round() as u8]);
        }
        Some(img)
    }
}

impl<'a> colr::Painter<'a> for ColrPainter<'_, 'a> {
    fn outline_glyph(&mut self, glyph_id: GlyphId) {
        let face = self.face;
        let mask = self.rasterize(|f| {
            face.outline_glyph(glyph_id, f);
        });
        self.outline = Some(mask);
    }

    fn paint(&mut self, paint: colr::Paint<'a>) {
        // COLRv0 paints straight into the last outline; COLRv1 paints fill the current clip.
        let outline = self.outline.take();
        let Some(inverse) = invert(self.ctm()) else {
            return;
        };
        let shader = Shader::new(&paint);
        let n = self.size;
        let clip = self.clips.last();
        let Some((layer, _)) = self.layers.last_mut() else {
            return;
        };
        for (i, dst) in layer.iter_mut().enumerate() {
            let mut cov = 1.0;
            if let Some(c) = clip {
                cov *= c[i];
            }
            if let Some(o) = &outline {
                cov *= o[i];
            }
            if cov <= 0.0 {
                continue;
            }
            let (px, py) = ((i % n) as f32 + 0.5, (i / n) as f32 + 0.5);
            let (x, y) = apply(&inverse, px, py);
            let Some(src) = shader.at(x, y) else {
                continue;
            };
            let src = src.scale(cov);
            *dst = src.add(dst.scale(1.0 - src.a));
        }
    }

    fn push_clip(&mut self) {
        let mask = self
            .outline
            .take()
            .unwrap_or_else(|| vec![0.0; self.size * self.size]);
        self.push_mask(mask);
    }

    fn push_clip_box(&mut self, clipbox: colr::ClipBox) {
        let mask = self.rasterize(|f| {
            f.move_to(clipbox.x_min, clipbox.y_min);
            f.line_to(clipbox.x_max, clipbox.y_min);
            f.line_to(clipbox.x_max, clipbox.y_max);
            f.line_to(clipbox.x_min, clipbox.y_max);
            f.close();
        });
        self.push_mask(mask);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn push_layer(&mut self, mode: colr::CompositeMode) {
        self.layers
            .push((vec![Px::default(); self.size * self.size], mode));
    }

    fn pop_layer(&mut self) {
        if self.layers.len() < 2 {
            return;
        }
        let Some((src, mode)) = self.layers.pop() else {
            return;
        };
        if let Some((dst, _)) = self.layers.last_mut() {
            for (d, s) in dst.iter_mut().zip(src) {
                *d = composite(mode, s, *d);
            }
        }
    }

    fn push_transform(&mut self, transform: Transform) {
        let next = Transform::combine(self.ctm(), transform);
        self.transforms.push(next);
    }

    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }
}

/// Feeds an outline into the rasteriser: applies the transform, flattens curves and keeps every
/// segment inside `0..=width` horizontally (the rasteriser's row-major accumulation buffer would
/// otherwise wrap stray coverage into neighbouring rows).
struct Flattener<'r> {
    raster: &'r mut Rasterizer,
    ctm: Transform,
    width: f32,
    start: Point,
    last: Point,
    open: bool,
}

impl Flattener<'_> {
    fn map(&self, x: f32, y: f32) -> Point {
        let (x, y) = apply(&self.ctm, x, y);
        point(x, y)
    }

    fn segment(&mut self, to: Point) {
        clipped_line(self.raster, self.last, to, self.width);
        self.last = to;
    }

    /// Curve as straight segments; `n` from the control polygon length in pixels.
    fn polyline(&mut self, pts: &[Point], eval: impl Fn(f32) -> Point) {
        let len: f32 = std::iter::once(self.last)
            .chain(pts.iter().copied())
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt())
            .sum();
        let n = if len.is_finite() {
            (len / 3.0).ceil().clamp(1.0, 64.0) as usize
        } else {
            1
        };
        for i in 1..=n {
            let p = eval(i as f32 / n as f32);
            self.segment(p);
        }
    }
}

impl OutlineBuilder for Flattener<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        let p = self.map(x, y);
        self.start = p;
        self.last = p;
        self.open = true;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.map(x, y);
        self.segment(p);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.last;
        let p1 = self.map(x1, y1);
        let p2 = self.map(x, y);
        self.polyline(&[p1, p2], |t| {
            let u = 1.0 - t;
            point(
                u * u * p0.x + 2.0 * u * t * p1.x + t * t * p2.x,
                u * u * p0.y + 2.0 * u * t * p1.y + t * t * p2.y,
            )
        });
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.last;
        let p1 = self.map(x1, y1);
        let p2 = self.map(x2, y2);
        let p3 = self.map(x, y);
        self.polyline(&[p1, p2, p3], |t| {
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            point(
                a * p0.x + b * p1.x + c * p2.x + d * p3.x,
                a * p0.y + b * p1.y + c * p2.y + d * p3.y,
            )
        });
    }

    fn close(&mut self) {
        if self.open {
            let start = self.start;
            self.segment(start);
            self.open = false;
        }
    }
}

/// Draw `p0 → p1` with the parts left of 0 or right of `width` pushed onto those edges. A
/// vertical edge at the border keeps the winding (and so the fill) of everything inside.
fn clipped_line(raster: &mut Rasterizer, p0: Point, p1: Point, width: f32) {
    if !(p0.x.is_finite() && p0.y.is_finite() && p1.x.is_finite() && p1.y.is_finite()) {
        return;
    }
    let mut cuts = [0.0f32, 1.0, 1.0, 1.0];
    let mut count = 1;
    let dx = p1.x - p0.x;
    if dx.abs() > f32::EPSILON {
        for edge in [0.0, width] {
            let t = (edge - p0.x) / dx;
            if t > 0.0 && t < 1.0 {
                cuts[count] = t;
                count += 1;
            }
        }
    }
    cuts[count] = 1.0;
    cuts[..=count].sort_by(f32::total_cmp);
    let at = |t: f32| point((p0.x + dx * t).clamp(0.0, width), p0.y + (p1.y - p0.y) * t);
    for w in cuts[..=count].windows(2) {
        raster.draw_line(at(w[0]), at(w[1]));
    }
}

fn apply(t: &Transform, x: f32, y: f32) -> (f32, f32) {
    (t.a * x + t.c * y + t.e, t.b * x + t.d * y + t.f)
}

fn invert(t: Transform) -> Option<Transform> {
    let det = t.a * t.d - t.b * t.c;
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    Some(Transform::new(
        t.d * inv,
        -t.b * inv,
        -t.c * inv,
        t.a * inv,
        (t.c * t.f - t.d * t.e) * inv,
        (t.b * t.e - t.a * t.f) * inv,
    ))
}

/// A paint resolved to plain numbers so it can be sampled per pixel.
enum Shader {
    Solid(Px),
    Linear {
        x0: f32,
        y0: f32,
        dx: f32,
        dy: f32,
        stops: Stops,
    },
    Radial {
        c0: (f32, f32),
        r0: f32,
        cd: (f32, f32),
        dr: f32,
        stops: Stops,
    },
    Sweep {
        cx: f32,
        cy: f32,
        start: f32,
        end: f32,
        stops: Stops,
    },
    None,
}

struct Stops {
    stops: Vec<(f32, Px)>,
    extend: colr::GradientExtend,
}

impl Stops {
    fn collect(stops: impl Iterator<Item = colr::ColorStop>, extend: colr::GradientExtend) -> Self {
        let mut stops: Vec<(f32, Px)> = stops
            .filter(|s| s.stop_offset.is_finite())
            .map(|s| (s.stop_offset, Px::from_rgba(s.color).to_linear()))
            .collect();
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        Stops { stops, extend }
    }

    /// Colour at `t`, premultiplied sRGB. The OpenType spec requires COLR gradients to be
    /// interpolated on premultiplied linear-light values, so stops are stored linear.
    fn at(&self, t: f32) -> Option<Px> {
        self.at_linear(t).map(Px::to_srgb)
    }

    fn at_linear(&self, t: f32) -> Option<Px> {
        let first = self.stops.first()?;
        let last = self.stops.last()?;
        if !t.is_finite() {
            return None;
        }
        let (lo, hi) = (first.0, last.0);
        let span = hi - lo;
        let t = match self.extend {
            colr::GradientExtend::Repeat if span > 1e-6 => lo + (t - lo).rem_euclid(span),
            colr::GradientExtend::Reflect if span > 1e-6 => {
                let u = (t - lo).rem_euclid(2.0 * span);
                lo + if u > span { 2.0 * span - u } else { u }
            }
            _ => t,
        };
        if t <= lo {
            return Some(first.1);
        }
        if t >= hi {
            return Some(last.1);
        }
        let i = self.stops.partition_point(|s| s.0 <= t);
        let (a, b) = (self.stops.get(i.checked_sub(1)?)?, self.stops.get(i)?);
        let w = b.0 - a.0;
        Some(if w > 1e-6 {
            a.1.lerp(b.1, (t - a.0) / w)
        } else {
            b.1
        })
    }
}

impl Shader {
    fn new(paint: &colr::Paint<'_>) -> Self {
        match paint {
            colr::Paint::Solid(c) => Shader::Solid(Px::from_rgba(*c)),
            colr::Paint::LinearGradient(g) => {
                let stops = Stops::collect(g.stops(0, &[]), g.extend);
                // The gradient runs from p0 towards p1, projected onto the normal of p0→p2.
                let (ax, ay) = (g.x1 - g.x0, g.y1 - g.y0);
                let (nx, ny) = (g.y2 - g.y0, -(g.x2 - g.x0));
                let nn = nx * nx + ny * ny;
                let (dx, dy) = if nn > 1e-6 {
                    let k = (ax * nx + ay * ny) / nn;
                    (nx * k, ny * k)
                } else {
                    (ax, ay)
                };
                let len2 = dx * dx + dy * dy;
                if len2 <= 1e-6 {
                    return Shader::None;
                }
                Shader::Linear {
                    x0: g.x0,
                    y0: g.y0,
                    dx: dx / len2,
                    dy: dy / len2,
                    stops,
                }
            }
            colr::Paint::RadialGradient(g) => Shader::Radial {
                c0: (g.x0, g.y0),
                r0: g.r0,
                cd: (g.x1 - g.x0, g.y1 - g.y0),
                dr: g.r1 - g.r0,
                stops: Stops::collect(g.stops(0, &[]), g.extend),
            },
            colr::Paint::SweepGradient(g) => Shader::Sweep {
                cx: g.center_x,
                cy: g.center_y,
                // Stored as half-turns biased by -1.0 (so +360° fits in F2DOT14), counter-
                // clockwise in y-up font space; `ttf_parser` hands over the raw value.
                start: (g.start_angle + 1.0) * 180.0,
                end: (g.end_angle + 1.0) * 180.0,
                stops: Stops::collect(g.stops(0, &[]), g.extend),
            },
        }
    }

    fn at(&self, x: f32, y: f32) -> Option<Px> {
        match self {
            Shader::Solid(c) => Some(*c),
            Shader::Linear {
                x0,
                y0,
                dx,
                dy,
                stops,
            } => stops.at((x - x0) * dx + (y - y0) * dy),
            Shader::Radial {
                c0,
                r0,
                cd,
                dr,
                stops,
            } => stops.at(radial_t(*c0, *r0, *cd, *dr, x, y)?),
            Shader::Sweep {
                cx,
                cy,
                start,
                end,
                stops,
            } => {
                let span = end - start;
                if span.abs() < 1e-6 {
                    return None;
                }
                let angle = (y - cy).atan2(x - cx).to_degrees().rem_euclid(360.0);
                stops.at((angle - start) / span)
            }
            Shader::None => None,
        }
    }
}

/// Largest `t` with `|p - c(t)| = r(t)` and `r(t) >= 0` for the two-point conical gradient
/// `c(t) = c0 + t·cd`, `r(t) = r0 + t·dr` (the canvas / COLRv1 definition).
fn radial_t(c0: (f32, f32), r0: f32, cd: (f32, f32), dr: f32, x: f32, y: f32) -> Option<f32> {
    let (px, py) = (x - c0.0, y - c0.1);
    let a = cd.0 * cd.0 + cd.1 * cd.1 - dr * dr;
    let b = px * cd.0 + py * cd.1 + r0 * dr;
    let c = px * px + py * py - r0 * r0;
    let valid = |t: f32| r0 + t * dr >= 0.0;
    if a.abs() < 1e-6 {
        if b.abs() < 1e-6 {
            return None;
        }
        let t = c / (2.0 * b);
        return valid(t).then_some(t);
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let (t1, t2) = ((b + root) / a, (b - root) / a);
    let (hi, lo) = if t1 > t2 { (t1, t2) } else { (t2, t1) };
    if valid(hi) {
        Some(hi)
    } else if valid(lo) {
        Some(lo)
    } else {
        None
    }
}

/// Composite premultiplied `s` (the popped layer) onto `d` with a `PaintComposite` mode.
fn composite(mode: colr::CompositeMode, s: Px, d: Px) -> Px {
    use colr::CompositeMode as M;
    let (sa, da) = (s.a, d.a);
    match mode {
        M::Clear => Px::default(),
        M::Source => s,
        M::Destination => d,
        M::SourceOver => s.add(d.scale(1.0 - sa)),
        M::DestinationOver => s.scale(1.0 - da).add(d),
        M::SourceIn => s.scale(da),
        M::DestinationIn => d.scale(sa),
        M::SourceOut => s.scale(1.0 - da),
        M::DestinationOut => d.scale(1.0 - sa),
        M::SourceAtop => s.scale(da).add(d.scale(1.0 - sa)),
        M::DestinationAtop => s.scale(1.0 - da).add(d.scale(sa)),
        M::Xor => s.scale(1.0 - da).add(d.scale(1.0 - sa)),
        M::Plus => {
            let p = s.add(d);
            Px {
                r: p.r.min(1.0),
                g: p.g.min(1.0),
                b: p.b.min(1.0),
                a: p.a.min(1.0),
            }
        }
        _ => {
            if sa <= 0.0 {
                return d;
            }
            if da <= 0.0 {
                return s;
            }
            // W3C compositing: blend on unpremultiplied colour, then source-over.
            let cs = s.rgb().map(|c| c / sa);
            let cb = d.rgb().map(|c| c / da);
            let blended = blend(mode, cs, cb);
            let mix = |i: usize, sc: f32, dc: f32| {
                (1.0 - da) * sc + (1.0 - sa) * dc + sa * da * blended[i]
            };
            Px {
                r: mix(0, s.r, d.r),
                g: mix(1, s.g, d.g),
                b: mix(2, s.b, d.b),
                a: sa + da - sa * da,
            }
        }
    }
}

fn blend(mode: colr::CompositeMode, cs: [f32; 3], cb: [f32; 3]) -> [f32; 3] {
    use colr::CompositeMode as M;
    let sep = |f: fn(f32, f32) -> f32| [f(cb[0], cs[0]), f(cb[1], cs[1]), f(cb[2], cs[2])];
    match mode {
        M::Multiply => sep(|b, s| b * s),
        M::Screen => sep(|b, s| b + s - b * s),
        M::Overlay => sep(|b, s| hard_light(s, b)),
        M::Darken => sep(f32::min),
        M::Lighten => sep(f32::max),
        M::ColorDodge => sep(|b, s| {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }),
        M::ColorBurn => sep(|b, s| {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }),
        M::HardLight => sep(hard_light),
        M::SoftLight => sep(|b, s| {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 {
                    ((16.0 * b - 12.0) * b + 4.0) * b
                } else {
                    b.sqrt()
                };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }),
        M::Difference => sep(|b, s| (b - s).abs()),
        M::Exclusion => sep(|b, s| b + s - 2.0 * b * s),
        M::Hue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
        M::Saturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
        M::Color => set_lum(cs, lum(cb)),
        M::Luminosity => set_lum(cb, lum(cs)),
        _ => cs,
    }
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b * 2.0 * s
    } else {
        let s2 = 2.0 * s - 1.0;
        b + s2 - b * s2
    }
}

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    let c = c.map(|v| v + d);
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    c.map(|v| {
        let mut v = v;
        if n < 0.0 && l - n > 1e-6 {
            v = l + (v - l) * l / (l - n);
        }
        if x > 1.0 && x - l > 1e-6 {
            v = l + (v - l) * (1.0 - l) / (x - l);
        }
        v
    })
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    if max - min <= 1e-6 {
        return [0.0; 3];
    }
    c.map(|v| (v - min) * s / (max - min))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(r: f32, g: f32, b: f32, a: f32) -> Px {
        Px { r, g, b, a }
    }

    #[test]
    fn srgb_round_trip() {
        for v in [0.0, 0.002, 0.2, 0.5, 0.9, 1.0] {
            assert!((linear_to_srgb(srgb_to_linear(v)) - v).abs() < 1e-5);
        }
        let half = px(0.5, 0.25, 0.0, 0.5).to_linear().to_srgb();
        assert!((half.r - 0.5).abs() < 1e-5 && (half.a - 0.5).abs() < 1e-6);
    }

    #[test]
    fn invert_round_trips() {
        let t = Transform::new(2.0, 0.5, -0.25, 3.0, 10.0, -4.0);
        let inv = invert(t).expect("invertible");
        let (x, y) = apply(&t, 7.0, -3.0);
        let (bx, by) = apply(&inv, x, y);
        assert!((bx - 7.0).abs() < 1e-4 && (by + 3.0).abs() < 1e-4);
        assert!(invert(Transform::new(0.0, 0.0, 0.0, 0.0, 1.0, 1.0)).is_none());
    }

    #[test]
    fn stops_pad_repeat_reflect() {
        // Stops are stored linear (pure red/blue are the same in both encodings).
        let red = px(1.0, 0.0, 0.0, 1.0);
        let blue = px(0.0, 0.0, 1.0, 1.0);
        let mut stops = Stops {
            stops: vec![(0.0, red), (1.0, blue)],
            extend: colr::GradientExtend::Pad,
        };
        let lo = stops.at(-1.0).expect("pad low");
        let hi = stops.at(2.0).expect("pad high");
        assert!((lo.r - 1.0).abs() < 1e-5 && lo.b.abs() < 1e-5);
        assert!((hi.b - 1.0).abs() < 1e-5 && hi.r.abs() < 1e-5);
        // Interpolated in linear light, then encoded back to sRGB (0.5 linear is ~0.735 sRGB).
        let mid = stops.at(0.5).expect("mid");
        assert!((mid.r - 0.7354).abs() < 1e-3 && (mid.b - 0.7354).abs() < 1e-3);
        stops.extend = colr::GradientExtend::Repeat;
        let r = stops.at(1.25).expect("repeat");
        assert!((r.r - linear_to_srgb(0.75)).abs() < 1e-5);
        stops.extend = colr::GradientExtend::Reflect;
        let r = stops.at(1.25).expect("reflect");
        assert!((r.r - linear_to_srgb(0.25)).abs() < 1e-5);
        assert!(stops.at(f32::NAN).is_none());
    }

    #[test]
    fn radial_t_concentric() {
        // Concentric circles r = 0 → 10: a point at distance 5 sits halfway.
        let t = radial_t((0.0, 0.0), 0.0, (0.0, 0.0), 10.0, 3.0, 4.0).expect("t");
        assert!((t - 0.5).abs() < 1e-5);
        // Shifted focal point: still finds the circle through the point.
        let t = radial_t((0.0, 0.0), 0.0, (10.0, 0.0), 20.0, 20.0, 0.0).expect("t");
        assert!(t > 0.0 && t <= 1.0);
    }

    #[test]
    fn porter_duff_modes() {
        let s = px(1.0, 0.0, 0.0, 1.0);
        let d = px(0.0, 0.0, 0.5, 0.5);
        assert_eq!(composite(colr::CompositeMode::SourceOver, s, d), s);
        assert_eq!(composite(colr::CompositeMode::DestinationOver, s, d).b, 0.5);
        assert_eq!(composite(colr::CompositeMode::SourceIn, s, d).a, 0.5);
        assert_eq!(
            composite(colr::CompositeMode::DestinationOut, s, d),
            Px::default()
        );
        let m = composite(colr::CompositeMode::Multiply, s, px(1.0, 1.0, 1.0, 1.0));
        assert!((m.r - 1.0).abs() < 1e-6 && m.g.abs() < 1e-6);
    }

    #[test]
    fn clipped_square_fills_once_even_when_off_canvas() {
        // A square much wider than the canvas must fill every pixel exactly once, not bleed
        // coverage into the next row.
        let mut raster = Rasterizer::new(8, 8);
        let pts = [
            (-20.0, 2.0),
            (30.0, 2.0),
            (30.0, 6.0),
            (-20.0, 6.0),
            (-20.0, 2.0),
        ];
        for w in pts.windows(2) {
            clipped_line(
                &mut raster,
                point(w[0].0, w[0].1),
                point(w[1].0, w[1].1),
                8.0,
            );
        }
        let mut cov = vec![0.0; 64];
        raster.for_each_pixel(|i, c| cov[i] = c);
        for (i, c) in cov.iter().enumerate() {
            let y = i / 8;
            let want = if (2..6).contains(&y) { 1.0 } else { 0.0 };
            assert!((c - want).abs() < 1e-4, "pixel {i}: {c}");
        }
    }
}
