//! Helper functions and types for the Adafruit PyPortal.
//!
//! Uses [`PollingSysTick`]{cortex_m_systick_countdown::PollingSysTick} as the
//! source of the `CountDown` instance.
use ::pyportal as hal;

use hal::clock::GenericClockController;
use hal::gpio;
use hal::pac;
use hal::pins;
// Does this belong in pyportal’s prelude.rs?
use hal::prelude::*;
use hal::sercom;
use hal::time::Hertz;

use cortex_m_systick_countdown::{CountsMillisCountDown, PollingSysTick, SysTickCalibration};

use core::time::Duration;

pub mod prelude;
use prelude::*;

/// Type for the internal ESP32 chip select pin.
pub type CsPin = gpio::Pb14<gpio::Output<gpio::PushPull>>;
/// Type for the internal ESP32 busy pin.
pub type BusyPin = gpio::Pb16<gpio::Input<gpio::Floating>>;
/// Type for the internal ESP32 reset pin.
pub type ResetPin = gpio::Pb17<gpio::Output<gpio::PushPull>>;

/// Type for SERCOM2 in SPI mode.
///
/// This matches the pins that are wired to the PyPortal’s onboard ESP32.
pub type Spi = sercom::SPIMaster2<
    sercom::Sercom2Pad2<gpio::Pa14<gpio::PfC>>,
    sercom::Sercom2Pad0<gpio::Pa12<gpio::PfC>>,
    sercom::Sercom2Pad1<gpio::Pa13<gpio::PfC>>,
>;

pub type SpiError = sercom::Error;
pub type Error = crate::Error<SpiError>;
pub type CountDown<'a> = CountsMillisCountDown<'a, PollingSysTick>;

pub type WifiNina<'cd> = crate::WifiNina<CsPin, BusyPin, Spi, CountDown<'cd>>;

pub type Socket<'a> = crate::Socket<'a, CsPin, Spi>;
pub type ServerSocket<'a> = crate::ServerSocket<'a, CsPin, Spi>;

pub type ConnectedSocket<'wifi, 's, 'cd> =
    crate::ConnectedSocket<'wifi, 's, CsPin, BusyPin, Spi, sercom::Error, CountDown<'cd>, Duration>;

/// Creates a PollingSysTick with the proper calibration for the PyPortal. This
/// will need to be passed in to make_wifi.
pub fn sys_tick(syst: pac::SYST, clocks: &mut GenericClockController) -> PollingSysTick {
    let clock_hertz: Hertz = clocks.gclk0().into();

    // PyPortal does not have a built-in calibration defined, so we use the
    // clock speed of the default clock, which matches the processor speed.
    let calibration = SysTickCalibration::from_clock_hz(clock_hertz.0);

    PollingSysTick::new(syst, &calibration)
}

/// Creates an SPI instance on SERCOM2, which is the bus that the ESP32 chip is
/// connected to on the PyPortal.
pub fn spi(
    clocks: &mut GenericClockController,
    sercom2: pac::SERCOM2,
    mclk: &mut pac::MCLK,
    port: &mut gpio::Port,
    spi: pins::Spi,
) -> Spi {
    // Because SPIs are a bus, we keep them separate from the WifiNina value and
    // pass them in for each method call, so they can be used for other devices.
    //
    // In the case of the PyPortal, the SD card and (optionally) the display are
    // on the same SPI bus.
    hal::spi_master(
        clocks,
        8000000.hz(),
        sercom2,
        mclk,
        spi.sck,
        spi.mosi,
        spi.miso,
        port,
    )
}

/// Creates a `WifiNina` instance for the PyPortal’s internal ESP32.
pub fn wifi<'cd>(
    port: &mut gpio::Port,
    esp: pins::Esp,
    spi: &Spi,
    sys_tick: &'cd PollingSysTick,
) -> Result<(WifiNina<'cd>, ResetPin), crate::Error<sercom::Error>> {
    let esp_cs = esp.cs.into_push_pull_output(port);
    let esp_busy = esp.busy.into_floating_input(port);
    let mut esp_reset = esp.reset.into_push_pull_output(port);

    Ok((
        WifiNina::new(
            spi,
            esp_cs,
            esp_busy,
            Some(&mut esp_reset),
            sys_tick.count_down(),
        )?,
        esp_reset,
    ))
}
