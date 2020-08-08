//! Helper functions and types for an AirLift FeatherWing on an Adafruit Feather
//! M4.
//!
//! Uses [`PollingSysTick`]{cortex_m_systick_countdown::PollingSysTick} as the
//! source of the `CountDown` instance.
use ::feather_m4 as hal;

use hal::clock::GenericClockController;
use hal::gpio;
use hal::pac;
// Does this belong in feather_m4’s prelude.rs?
use hal::prelude::*;
use hal::sercom;
use hal::time::Hertz;

use cortex_m_systick_countdown::{CountsMillisCountDown, PollingSysTick, SysTickCalibration};

use core::time::Duration;

pub mod prelude;
use prelude::*;

/// Type for D13, which is what the AirLift matches to CS.
pub type CsPin = gpio::Pa23<gpio::Output<gpio::PushPull>>;
/// Type for D11, which is what the AirLift matches to Busy.
pub type BusyPin = gpio::Pa21<gpio::Input<gpio::Floating>>;
/// Type for D12, which is what the AirLift matches to Reset.
pub type ResetPin = gpio::Pa22<gpio::Output<gpio::PushPull>>;

/// Type for SERCOM1 in SPI mode.
///
/// This matches the standard Feather SPI pins.
pub type Spi = sercom::SPIMaster1<
    hal::sercom::Sercom1Pad2<gpio::Pb22<gpio::PfC>>,
    hal::sercom::Sercom1Pad3<gpio::Pb23<gpio::PfC>>,
    hal::sercom::Sercom1Pad1<gpio::Pa17<gpio::PfC>>,
>;

pub type SpiError = sercom::Error;
pub type Error = crate::Error<SpiError>;
pub type CountDown<'a> = CountsMillisCountDown<'a, PollingSysTick>;

pub type WifiNina<'cd> = crate::WifiNina<CsPin, BusyPin, Spi, CountDown<'cd>>;

pub type Socket<'a> = crate::Socket<'a, CsPin, Spi>;
pub type ServerSocket<'a> = crate::ServerSocket<'a, CsPin, Spi>;

pub type ConnectedSocket<'wifi, 's, 'cd> =
    crate::ConnectedSocket<'wifi, 's, CsPin, BusyPin, Spi, sercom::Error, CountDown<'cd>, Duration>;

/// Creates a PollingSysTick with the proper calibration for the Feather M4. This
/// will need to be passed in to make_wifi.
pub fn sys_tick(syst: pac::SYST, clocks: &mut GenericClockController) -> PollingSysTick {
    let clock_hertz: Hertz = clocks.gclk0().into();

    // Feathre M4 does not have a built-in calibration defined, so we use the
    // clock speed of the default clock, which matches the processor speed.
    let calibration = SysTickCalibration::from_clock_hz(clock_hertz.0);

    PollingSysTick::new(syst, &calibration)
}

/// Creates an SPI instance on SERCOM1, which corresponds to the standard
/// Feather SPI pins.
pub fn spi(
    clocks: &mut GenericClockController,
    sercom1: pac::SERCOM1,
    mclk: &mut pac::MCLK,
    port: &mut gpio::Port,
    sck: gpio::Pa17<gpio::Input<gpio::Floating>>,
    mosi: gpio::Pb23<gpio::Input<gpio::Floating>>,
    miso: gpio::Pb22<gpio::Input<gpio::Floating>>,
) -> Spi {
    // Because SPIs are a bus, we keep them separate from the WifiNina value and
    // pass them in for each method call, so they can be used for other devices.
    hal::spi_master(clocks, 8000000.hz(), sercom1, mclk, sck, mosi, miso, port)
}

/// Creates a `WifiNina` instance for the PyPortal’s internal ESP32.
pub fn wifi<'cd>(
    port: &mut gpio::Port,
    d11: gpio::Pa21<gpio::Input<gpio::Floating>>,
    d12: gpio::Pa22<gpio::Input<gpio::Floating>>,
    d13: gpio::Pa23<gpio::Input<gpio::Floating>>,
    spi: &Spi,
    sys_tick: &'cd PollingSysTick,
) -> Result<(WifiNina<'cd>, ResetPin), crate::Error<sercom::Error>> {
    let esp_cs = d13.into_push_pull_output(port);
    let esp_busy = d11.into_floating_input(port);
    let mut esp_reset = d12.into_push_pull_output(port);

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
