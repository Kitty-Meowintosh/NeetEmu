//! Packed-RGB surfaces and the operations on them, ported from `APIS/graphics/GraphicalAPI.java`.

/// An `ExposedError` or a `RangeArgumentError`, raised verbatim by the binding.
#[derive(Debug, PartialEq, Eq)]
pub enum GfxError {
    Exposed(String),
    /// `position`, `min`, `max`, `value`.
    Range(i32, i64, i64, i64),
}

type Result<T> = std::result::Result<T, GfxError>;

fn exposed<T>(msg: &str) -> Result<T> {
    Err(GfxError::Exposed(msg.into()))
}

/// `GraphicalAPI.pack` — `0xRRGGBBAA`.
pub fn pack(red: i64, green: i64, blue: i64, alpha: i64) -> u32 {
    ((red as u32) << 24 & 0xFF00_0000)
        | ((green as u32) << 16 & 0x00FF_0000)
        | ((blue as u32) << 8 & 0x0000_FF00)
        | (alpha as u32 & 0xFF)
}

/// `GraphicalAPI.blend` — `rgb` is `0xRRGGBB`, `rgba` is `0xRRGGBBAA`.
pub fn blend(rgb: u32, rgba: u32) -> u32 {
    let a = (rgba & 0xFF) as u64;
    let src = (rgba >> 8) as u64;
    let dst = rgb as u64;
    let channel = |mask: u64| -> u32 {
        (((src & mask) * a + (dst & mask) * (255 - a)) / 255) as u32 & mask as u32
    };
    channel(0xFF_0000) | channel(0xFF00) | channel(0xFF)
}

/// Source-over of `0xRRGGBB` at `sa` onto `d` at `da`; the colour and alpha of the result.
fn over(s: u32, sa: u32, d: u32, da: u32) -> (u32, u32) {
    let back = da * (255 - sa) / 255;
    let oa = sa + back;
    if oa == 0 {
        return (d, 0);
    }
    let channel = |shift: u32| -> u32 {
        let sc = (s >> shift) & 0xFF;
        let dc = (d >> shift) & 0xFF;
        ((sc * sa + dc * back) / oa).min(255) << shift
    };
    (channel(16) | channel(8) | channel(0), oa)
}

/// One drawable surface: the screen itself, or a layer.
#[derive(Clone)]
pub struct Surface {
    width: u32,
    height: u32,
    buffer: Vec<u32>,
    /// Per-pixel alpha, a NeetEmu extension present only on a layer created transparent.
    alpha: Option<Vec<u8>>,
}

impl Surface {
    pub fn new(width: u32, height: u32) -> Surface {
        Surface {
            width,
            height,
            buffer: vec![0; (width * height) as usize],
            alpha: None,
        }
    }

    /// A layer with an alpha plane, every pixel transparent.
    pub fn new_transparent(width: u32, height: u32) -> Surface {
        let mut surface = Surface::new(width, height);
        surface.alpha = Some(vec![0; (width * height) as usize]);
        surface
    }

    /// Writes one pixel of `0xRRGGBB` at alpha `a`: as given when `replace`, blended otherwise.
    fn put(&mut self, slot: usize, rgb: u32, a: u32, replace: bool) {
        match self.alpha.as_mut() {
            None => {
                self.buffer[slot] = match (replace, a) {
                    (true, _) | (_, 0xFF) => rgb,
                    (_, 0) => self.buffer[slot],
                    _ => blend(self.buffer[slot], rgb << 8 | a),
                };
            }
            Some(plane) => {
                if replace || a == 0xFF {
                    self.buffer[slot] = rgb;
                    plane[slot] = a as u8;
                } else if a != 0 {
                    let (c, oa) = over(rgb, a, self.buffer[slot], plane[slot] as u32);
                    self.buffer[slot] = c;
                    plane[slot] = oa as u8;
                }
            }
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The packed `0xRRGGBB` store, for the display to present.
    pub fn pixels(&self) -> &[u32] {
        &self.buffer
    }

    fn at(&self, x: u32, y: u32) -> usize {
        (x + y * self.width) as usize
    }

    /// The `x1 <= x2`, in-bounds check shared by `clone`, `readData` and `fill`, in upstream's wording.
    fn check_rect(&self, x1: i64, y1: i64, x2: i64, y2: i64) -> Result<()> {
        if x1 > x2 {
            return exposed("x2 must be larger then x1");
        }
        if y1 > y2 {
            return exposed("y2 must be larger then y1");
        }
        let (w, h) = (self.width as i64, self.height as i64);
        if x1 < 0 {
            return Err(GfxError::Range(0, 0, w - 1, x1));
        }
        if y1 < 0 {
            return Err(GfxError::Range(1, 0, h - 1, y1));
        }
        if x2 >= w {
            return Err(GfxError::Range(2, 0, w - 1, x2));
        }
        if y2 >= h {
            return Err(GfxError::Range(3, 0, h - 1, y2));
        }
        Ok(())
    }

    /// Clips silently, unlike the rect operations.
    pub fn write_pixel(&mut self, x: i64, y: i64, r: i64, g: i64, b: i64, alpha: Option<i64>) {
        if x < 0 || x >= self.width as i64 || y < 0 || y >= self.height as i64 {
            return;
        }
        let rgba = pack(r, g, b, alpha.unwrap_or(255));
        let i = self.at(x as u32, y as u32);
        self.put(i, rgba >> 8, rgba & 0xFF, false);
    }

    /// Returns the three channels.
    pub fn read_pixel(&self, x: i64, y: i64) -> Result<(u32, u32, u32)> {
        if x < 0 || x >= self.width as i64 {
            return Err(GfxError::Range(0, 0, self.width as i64 - 1, x));
        }
        if y < 0 || y >= self.height as i64 {
            return Err(GfxError::Range(1, 0, self.height as i64 - 1, y));
        }
        let rgb = self.buffer[self.at(x as u32, y as u32)];
        Ok(((rgb & 0xFF_0000) >> 16, (rgb & 0xFF00) >> 8, rgb & 0xFF))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write_line(
        &mut self,
        x1: i64,
        y1: i64,
        x2: i64,
        y2: i64,
        r: i64,
        g: i64,
        b: i64,
        alpha: Option<i64>,
    ) {
        let (dx, dy) = ((x2 - x1) as f32, (y2 - y1) as f32);
        let step = dx.abs().max(dy.abs());
        let (x_incr, y_incr) = (dx / step, dy / step);
        let (mut x, mut y) = (x1 as f32, y1 as f32);
        for _ in 0..=(step as i64) {
            self.write_pixel(java_round(x), java_round(y), r, g, b, alpha);
            x += x_incr;
            y += y_incr;
        }
    }

    /// Swaps one packed colour for another; alpha is dropped on both sides.
    pub fn substitute(&mut self, r1: i64, g1: i64, b1: i64, r2: i64, g2: i64, b2: i64) {
        let src = pack(r1, g1, b1, 0) >> 8;
        let dest = pack(r2, g2, b2, 0) >> 8;
        for px in &mut self.buffer {
            if *px == src {
                *px = dest;
            }
        }
    }

    /// The packed pixels of an inclusive rectangle, row-major.
    fn read_sector(&self, x1: u32, y1: u32, x2: u32, y2: u32) -> Vec<u32> {
        let w = (x2 - x1 + 1) as usize;
        let mut out = Vec::with_capacity(w * (y2 - y1 + 1) as usize);
        for y in y1..=y2 {
            let start = self.at(x1, y);
            out.extend_from_slice(&self.buffer[start..start + w]);
        }
        out
    }

    pub fn clone_rect(&self, x1: i64, y1: i64, x2: i64, y2: i64) -> Result<Surface> {
        self.check_rect(x1, y1, x2, y2)?;
        let (x1, y1, x2, y2) = (x1 as u32, y1 as u32, x2 as u32, y2 as u32);
        Ok(Surface {
            width: x2 - x1 + 1,
            height: y2 - y1 + 1,
            buffer: self.read_sector(x1, y1, x2, y2),
            alpha: self
                .alpha
                .as_ref()
                .map(|plane| self.alpha_sector(plane, x1, y1, x2, y2)),
        })
    }

    /// The alpha of an inclusive rectangle, row-major.
    fn alpha_sector(&self, plane: &[u8], x1: u32, y1: u32, x2: u32, y2: u32) -> Vec<u8> {
        let w = (x2 - x1 + 1) as usize;
        let mut out = Vec::with_capacity(w * (y2 - y1 + 1) as usize);
        for y in y1..=y2 {
            let start = self.at(x1, y);
            out.extend_from_slice(&plane[start..start + w]);
        }
        out
    }

    /// Four bytes per pixel over an inclusive rectangle, alpha `0xFF` unless the layer keeps an alpha plane.
    pub fn read_data(&self, x1: i64, y1: i64, x2: i64, y2: i64) -> Result<Vec<u8>> {
        self.check_rect(x1, y1, x2, y2)?;
        let (ux1, uy1, ux2, uy2) = (x1 as u32, y1 as u32, x2 as u32, y2 as u32);
        let sector = self.read_sector(ux1, uy1, ux2, uy2);
        let alphas = self
            .alpha
            .as_ref()
            .map(|plane| self.alpha_sector(plane, ux1, uy1, ux2, uy2));
        let mut out = Vec::with_capacity(sector.len() * 4);
        for (i, px) in sector.into_iter().enumerate() {
            let a = alphas.as_ref().map_or(0xFF, |a| a[i]);
            out.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, px as u8, a]);
        }
        Ok(out)
    }

    /// Writes a buffer whose height follows from its length, refusing one that overhangs, blending unless `replace`.
    pub fn write_data(
        &mut self,
        x: i64,
        y: i64,
        data: &[u8],
        width: i64,
        replace: bool,
    ) -> Result<()> {
        if !data.len().is_multiple_of(4) {
            return exposed("Length of buffer must by dividable by 4");
        }
        if width <= 0 || !(data.len() / 4).is_multiple_of(width as usize) {
            return exposed("Length of buffer must by dividable by width");
        }
        let height = data.len() as i64 / width / 4;
        // The bound reported here is the buffer's, not the surface's.
        if x < 0 {
            return Err(GfxError::Range(0, 0, width - 1, x));
        }
        if y < 0 {
            return Err(GfxError::Range(1, 0, height - 1, y));
        }
        if y + height > self.height as i64 || x + width > self.width as i64 {
            return exposed("Draw call extends past valid bounds");
        }

        let (x, y, width, height) = (x as u32, y as u32, width as u32, height as u32);
        for row in 0..height {
            let dst = self.at(x, y + row);
            for col in 0..width {
                let i = ((row * width + col) * 4) as usize;
                let (r, g, b, a) = (
                    data[i] as u32,
                    data[i + 1] as u32,
                    data[i + 2] as u32,
                    data[i + 3] as u32,
                );
                self.put(dst + col as usize, r << 16 | g << 8 | b, a, replace);
            }
        }
        Ok(())
    }

    /// `set()` — clears to zero, transparent on a layer with an alpha plane.
    pub fn clear(&mut self) {
        self.buffer.fill(0);
        if let Some(plane) = self.alpha.as_mut() {
            plane.fill(0);
        }
    }

    /// `set(r, g, b, a?)`; opaque black clears, and any opaque colour fills.
    pub fn set(&mut self, r: i64, g: i64, b: i64, alpha: Option<i64>) {
        let rgba = pack(r, g, b, alpha.unwrap_or(255));
        if rgba == 0xFF {
            self.clear();
            return;
        }
        if rgba & 0xFF == 255 {
            self.buffer.fill(rgba >> 8);
            if let Some(plane) = self.alpha.as_mut() {
                plane.fill(0xFF);
            }
            return;
        }
        for i in 0..self.buffer.len() {
            self.put(i, rgba >> 8, rgba & 0xFF, false);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill(
        &mut self,
        x1: i64,
        y1: i64,
        x2: i64,
        y2: i64,
        r: i64,
        g: i64,
        b: i64,
        alpha: Option<i64>,
        replace: bool,
    ) -> Result<()> {
        self.check_rect(x1, y1, x2, y2)?;
        let rgba = pack(r, g, b, alpha.unwrap_or(255));
        let (x1, y1, x2, y2) = (x1 as u32, y1 as u32, x2 as u32, y2 as u32);
        let (rgb, a) = (rgba >> 8, rgba & 0xFF);
        let whole = replace || a == 0xFF;
        let w = (x2 - x1) as usize;
        for y in y1..=y2 {
            let start = self.at(x1, y);
            if whole {
                self.buffer[start..=start + w].fill(rgb);
                if let Some(plane) = self.alpha.as_mut() {
                    plane[start..=start + w].fill(a as u8);
                }
            } else {
                for slot in start..=start + w {
                    self.put(slot, rgb, a, false);
                }
            }
        }
        Ok(())
    }
}

/// `Math.round(float)`, which is `floor(x + 0.5)`.
fn java_round(v: f32) -> i64 {
    if v.is_nan() {
        return 0;
    }
    (v + 0.5).floor() as i64
}

/// Every surface a machine owns, slot 0 the screen.
pub struct Surfaces {
    surfaces: Vec<Surface>,
}

impl Surfaces {
    pub fn new(width: u32, height: u32) -> Surfaces {
        Surfaces {
            surfaces: vec![Surface::new(width, height)],
        }
    }

    pub fn screen(&self) -> &Surface {
        &self.surfaces[0]
    }

    pub fn get(&self, id: usize) -> Option<&Surface> {
        self.surfaces.get(id)
    }

    pub fn get_mut(&mut self, id: usize) -> Option<&mut Surface> {
        self.surfaces.get_mut(id)
    }

    /// `createLayer`, present on every layer.
    pub fn create(&mut self, width: i64, height: i64, transparent: bool) -> Result<usize> {
        if width <= 0 || height <= 0 {
            return exposed("Size cant be zero or less");
        }
        let (w, h) = (width as u32, height as u32);
        self.surfaces.push(if transparent {
            Surface::new_transparent(w, h)
        } else {
            Surface::new(w, h)
        });
        Ok(self.surfaces.len() - 1)
    }

    pub fn insert(&mut self, surface: Surface) -> usize {
        self.surfaces.push(surface);
        self.surfaces.len() - 1
    }

    pub fn len(&self) -> usize {
        self.surfaces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }
}

#[cfg(test)]
mod alpha_tests {
    use super::*;

    #[test]
    fn a_transparent_layer_reads_back_transparent() {
        let layer = Surface::new_transparent(2, 1);
        assert_eq!(
            layer.read_data(0, 0, 1, 0).unwrap(),
            vec![0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn a_translucent_write_keeps_its_alpha_on_a_transparent_layer() {
        let mut layer = Surface::new_transparent(1, 1);
        layer.write_data(0, 0, &[255, 0, 0, 128], 1, false).unwrap();
        assert_eq!(layer.read_data(0, 0, 0, 0).unwrap(), vec![255, 0, 0, 128]);
    }

    #[test]
    fn replace_stores_the_pixel_as_given() {
        let mut layer = Surface::new_transparent(1, 1);
        layer.fill(0, 0, 0, 0, 0, 0, 255, Some(255), false).unwrap();
        layer.write_data(0, 0, &[10, 20, 30, 40], 1, true).unwrap();
        assert_eq!(layer.read_data(0, 0, 0, 0).unwrap(), vec![10, 20, 30, 40]);
        layer.fill(0, 0, 0, 0, 0, 0, 0, Some(0), true).unwrap();
        assert_eq!(layer.read_data(0, 0, 0, 0).unwrap(), vec![0, 0, 0, 0]);
    }

    #[test]
    fn an_opaque_layer_still_reads_back_opaque() {
        let mut layer = Surface::new(1, 1);
        layer.write_data(0, 0, &[255, 0, 0, 128], 1, false).unwrap();
        assert_eq!(layer.read_data(0, 0, 0, 0).unwrap()[3], 0xFF);
    }
}
