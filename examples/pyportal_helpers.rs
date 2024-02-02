use pyportal as hal;

use embedded_hal::digital::v1_compat::OldOutputPin;

use hal::clock::{GClock, GenericClockController};
use hal::gpio;
use hal::pac;
use hal::sercom;
use hal::sercom::PadPin;
use hal::time::Hertz;
use hal::timer::SpinTimer;

use core::fmt::Write;

use ws2812_timer_delay::Ws2812;

/// Creates a UART for the 4-pin JST STEMMA connector on the left side of the
/// PyPortal. Used to write debug and logging information, but not necessary for
/// seeing this example run.
///
/// Runs at 115200 baud.
pub fn stemma_uart(
    sercom5: pac::SERCOM5,
    clocks: &mut GenericClockController,
    gclock: &GClock,
    mclk: &mut pac::MCLK,
    port: &mut gpio::Port,
    scl: gpio::Pb3<gpio::Input<gpio::Floating>>,
    sda: gpio::Pb2<gpio::Input<gpio::Floating>>,
) -> sercom::UART5<
    sercom::Sercom5Pad1<gpio::Pb3<gpio::PfD>>,
    sercom::Sercom5Pad0<gpio::Pb2<gpio::PfD>>,
    (),
    (),
> {
    let tx: sercom::Sercom5Pad1<_> = scl.into_pull_down_input(port).into_pad(port);
    let rx: sercom::Sercom5Pad0<_> = sda.into_pull_down_input(port).into_pad(port);

    let uart_clk = clocks
        .sercom5_core(gclock)
        .expect("Could not configure sercom5 clock");

    sercom::UART5::new(&uart_clk, Hertz(115200), sercom5, mclk, (tx, rx))
}

/// Creates a Ws2812 to control the PyPortal’s onboard NeoPixel.
///
/// You’ll likely need to have `use smart_leds::SmartLedsWrite;`
pub fn onboard_neopixel(
    port: &mut gpio::Port,
    neopixel: gpio::Pb22<gpio::Input<gpio::Floating>>,
) -> Ws2812<
    SpinTimer,
    embedded_hal::digital::v1_compat::OldOutputPin<gpio::Pb22<gpio::Output<gpio::PushPull>>>,
> {
    let neopixel_pin: OldOutputPin<_> = neopixel.into_push_pull_output(port).into();
    let timer = SpinTimer::new(4);

    Ws2812::new(timer, neopixel_pin)
}
