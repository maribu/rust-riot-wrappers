//! Wrappers for the [microbit module] exposing the micro:bit LED display
//!
//! [microbit module]: https://doc.riot-os.org/group__boards__common__microbit.html


use embedded_graphics::{DrawTarget, pixelcolor::BinaryColor, geometry::Size, drawable::Pixel, geometry::Point};

/// The 5x5 LED matrix of the micro:bit boards
///
/// Use the [embedded_hal] mechanisms to paint on them.
pub struct LEDs {
    _private: ()
}

impl LEDs {
    /// Initialize the micro:bit LEDs
    pub fn new() -> Self {
        unsafe { riot_sys::microbit_matrix_init() };
        Self { _private: () }
    }
}

impl DrawTarget<BinaryColor> for LEDs {
    type Error = !;

    fn draw_pixel(&mut self, pixel: Pixel<BinaryColor>) -> Result<(), !> {
        let Pixel(Point { x, y }, color) = pixel;

        let setter = match color {
            BinaryColor::On => riot_sys::microbit_matrix_on,
            BinaryColor::Off => riot_sys::microbit_matrix_off,
        };
        unsafe { setter(y as _, x as _) };

        Ok(())
    }

    fn size(&self) -> Size {
        Size::new(riot_sys::MICROBIT_MATRIX_COLS, riot_sys::MICROBIT_MATRIX_ROWS)
    }
}
