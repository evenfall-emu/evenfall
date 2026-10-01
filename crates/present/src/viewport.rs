/// The part of the drawable area a picture covers, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    /// Places a `source` picture (width, height) in a `drawable` area
    /// (width, height) at the source's aspect ratio, centred, assuming square
    /// source pixels.
    ///
    /// The picture fills the area along its limiting axis, and its other side
    /// is rounded to the nearest pixel, halves up. With `integer_scale` it is
    /// the largest whole multiple of the source that fits instead, unless not
    /// even one does. Any leftover splits evenly, with the odd pixel on the
    /// right or bottom.
    pub fn fit(source: (u32, u32), drawable: (u32, u32), integer_scale: bool) -> Self {
        let (sw, sh) = (u64::from(source.0), u64::from(source.1));
        let (dw, dh) = (u64::from(drawable.0), u64::from(drawable.1));
        if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
            return Self { x: 0, y: 0, width: 0, height: 0 };
        }
        let multiple = (dw / sw).min(dh / sh);
        let (width, height) = if integer_scale && multiple >= 1 {
            (sw * multiple, sh * multiple)
        } else if dw * sh <= dh * sw {
            (dw, (2 * dw * sh + sw) / (2 * sw))
        } else {
            ((2 * dh * sw + sh) / (2 * sh), dh)
        };
        Self {
            x: ((dw - width) / 2) as u32,
            y: ((dh - height) / 2) as u32,
            width: width as u32,
            height: height as u32,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

#[cfg(test)]
mod tests {
    use super::Viewport;

    const GBA: (u32, u32) = (240, 160);

    fn viewport(x: u32, y: u32, width: u32, height: u32) -> Viewport {
        Viewport { x, y, width, height }
    }

    /// A drawable area that is an exact multiple is filled, with or without
    /// integer scaling.
    #[test]
    fn exact_multiple_fills_the_area() {
        for integer in [false, true] {
            assert_eq!(Viewport::fit(GBA, (720, 480), integer), viewport(0, 0, 720, 480));
        }
    }

    /// 1000x700 holds 4x (960x640) with 20 and 30 pixels to spare on each side,
    /// while fitting fills the width and rounds 666.67 rows up to 667.
    #[test]
    fn integer_scale_keeps_whole_multiples_and_fitting_fills_one_axis() {
        assert_eq!(Viewport::fit(GBA, (1000, 700), true), viewport(20, 30, 960, 640));
        assert_eq!(Viewport::fit(GBA, (1000, 700), false), viewport(0, 16, 1000, 667));
    }

    /// A wide 1920x1080 area limits the height: 1080 rows need 1620 columns,
    /// centred with 150 to spare on each side. The rows also limit the whole
    /// multiple to 6x (1080 / 160 = 6.75), even though the width fits 8x.
    #[test]
    fn wide_area_limits_the_height() {
        assert_eq!(Viewport::fit(GBA, (1920, 1080), false), viewport(150, 0, 1620, 1080));
        assert_eq!(Viewport::fit(GBA, (1920, 1080), true), viewport(240, 60, 1440, 960));
    }

    /// A phone held upright limits the width, and 393 columns need 262 rows
    /// (262.0 exactly).
    #[test]
    fn tall_area_limits_the_width() {
        assert_eq!(Viewport::fit(GBA, (393, 852), false), viewport(0, 295, 393, 262));
        assert_eq!(Viewport::fit(GBA, (393, 852), true), viewport(76, 346, 240, 160));
    }

    /// An area smaller than the source can't hold one whole multiple, so
    /// integer scaling falls back to fitting: 200 columns need 133.33 rows.
    #[test]
    fn integer_scale_falls_back_below_one_multiple() {
        assert_eq!(Viewport::fit(GBA, (200, 200), true), viewport(0, 33, 200, 133));
    }

    /// Odd leftovers put the extra pixel after the picture, so a 3-pixel
    /// border is 1 on the left and 2 on the right.
    #[test]
    fn odd_leftover_goes_right_and_down() {
        assert_eq!(Viewport::fit((2, 2), (7, 4), true), viewport(1, 0, 4, 4));
        assert_eq!(Viewport::fit((2, 2), (4, 7), true), viewport(0, 1, 4, 4));
    }

    #[test]
    fn empty_area_gives_an_empty_viewport() {
        assert!(Viewport::fit(GBA, (0, 480), false).is_empty());
        assert!(Viewport::fit(GBA, (720, 0), true).is_empty());
    }
}
