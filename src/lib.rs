//! This library connects to WiFiNINA-based ESP32 Wi-Fi coprocessors over SPI.
//!
//! WiFiNINA originates in the Arduino project. These co-processors are also
//! marketed as “AirLift” by Adafruit and are found in the Adafruit PyPortal.
//!
//! See the [`WifiNina`](struct.WifiNina.html) struct.
//!
//! This crate contains setup helpers for the PyPortal and the AirLift
//! FeatherWing. These are enabled with the `device-pyportal` and
//! `device-featherwing` features, respectively, and found in the
//! [`pyportal`](pyportal/index.html) and featherwing modules.
//!
//! It also has `no_std` wrappers for parsing HTTP request and response headers,
//! available with the `http` feature and [`http`](http/index.html) module.
//!
//! If you use [`genio`](https://docs.rs/genio/)’s [`io`](std::io) replacements,
//! you can use the `genio-traits` feature to generate [`Read`](genio::Read) and
//! [`Write`](genio::Write) implementations for
//! [`ConnectedSocket`](struct.ConnectedSocket.html).
//!
//! Take a look at the **Examples** for how to initialize and use the library.

#![no_std]

mod chip_select;
mod commands;
mod util;

#[cfg(feature = "http")]
pub mod http;

#[cfg(feature = "device-pyportal")]
pub mod pyportal;

#[cfg(feature = "device-feather-m4")]
pub mod feather_m4;

use core::time::Duration;

use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::spi::FullDuplex;

use nb::block;

use chip_select::*;

pub use commands::{
    socket::{ConnectedSocket, Destination, Protocol, ServerSocket, Socket, SocketStatus},
    wifi::{WifiScanResults, WifiStatus},
};

/// Device interface for the WiFiNINA ESP32 wi-fi co-processor found in the
/// PyPortal, AirLift FeatherWing, and other places.
///
/// **References:**
/// * [Firmware source code (Arduino)](https://github.com/arduino/nina-fw)
/// * [Firmware source code (Adafruit
///   fork)](https://github.com/adafruit/nina-fw)
/// * [CircuitPython
///   library](https://github.com/adafruit/Adafruit_CircuitPython_ESP32SPI)
///
/// **Warning**: As of this writing, we don’t distinguish between the Adafruit
/// and Arduino implementations, since this code is only tested on a PyPortal
/// and a FeatherWing. PRs welcome.
///
/// To create this struct, you’ll need:
///
/// * The CS [`OutputPin`](embedded_hal::digital::v2::OutputPin) for the ESP32
/// * The busy [`InputPin`](embedded_hal::digital::v2::InputPin) for the ESP32
/// * A [`FullDuplex`](embedded_hal::spi::FullDuplex) SERCOM SPI instance
/// * A [`CountDown`](embedded_hal::timer::CountDown) instance
/// * (Optional) The reset [`OutputPin`](embedded_hal::digital::v2::OutputPin)
///   for the ESP32
///
/// This struct takes ownership of the CS and busy pins as well as the
/// `CountDown`. You’ll pass a mutable reference to the SPI bus when you call
/// each of the methods. `WiFiNina` will select its own chip, but it’s your
/// responsibility to make sure that no other chips are also selected on the SPI
/// bus.
///
/// If you’re on a Cortex-M device, consider using
/// [`cortex-m-systick-countdown`](https://docs.rs/cortex-m-systick-countdown)
/// for a shareable source of `CountDown` instance.
pub struct WifiNina<CsPin, BusyPin, Spi, CountDown>
where
    CsPin: OutputPin,
    BusyPin: InputPin,
{
    spi: core::marker::PhantomData<Spi>,
    chip_select: WifiNinaChipSelect<Spi, CsPin, BusyPin>,
    timer: CountDown,
}

impl<CsPin, BusyPin, Spi, SpiError, CountDown, CountDownTime>
    WifiNina<CsPin, BusyPin, Spi, CountDown>
where
    BusyPin: InputPin,
    CsPin: OutputPin,
    SpiError: core::fmt::Debug,
    Spi: FullDuplex<u8, Error = SpiError>
        + embedded_hal::blocking::spi::Write<u8, Error = SpiError>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SpiError>,
    CountDown: embedded_hal::timer::CountDown<Time = CountDownTime>,
    CountDownTime: From<Duration>,
{
    /// Creates a `WifiNina` instance.
    ///
    /// Does not use or save the SPI instance, but takes it as an argument so
    /// that we can infer its type.
    ///
    /// Implicitly calls reset if the reset pin is provided.
    pub fn new<ResetPin>(
        _spi: &Spi,
        cs: CsPin,
        busy: BusyPin,
        reset: Option<&mut ResetPin>,
        timer: CountDown,
    ) -> Result<Self, Error<SpiError>>
    where
        ResetPin: OutputPin,
    {
        let mut wifi = WifiNina {
            spi: core::marker::PhantomData,
            chip_select: WifiNinaChipSelect::new(cs, busy)
                .map_err(|_| Error::ChipSelectPinError)?,
            timer,
        };

        if let Some(r) = reset {
            wifi.reset(r)?;
        }

        Ok(wifi)
    }

    /// Reboots the WiFiNINA chip by bringing the reset pin low for 200ms.
    pub fn reset<ResetPin>(&mut self, reset: &mut ResetPin) -> Result<(), Error<SpiError>>
    where
        ResetPin: OutputPin,
    {
        reset.set_low().map_err(|_| Error::ResetPinError)?;

        self.timer.start(Duration::from_millis(200));
        block!(self.timer.wait()).unwrap();

        reset.set_high().map_err(|_| Error::ResetPinError)?;

        // Give the chip time to start back up.
        self.timer.start(Duration::from_millis(750));
        block!(self.timer.wait()).unwrap();

        Ok(())
    }
}

#[derive(Debug)]
pub enum Error<SpiError> {
    /// There was an I/O error writing to the CS pin. Really won’t happen unless
    /// the pin is on a GPIO expander or something, but it’s a part of the
    /// [`v2::OutputPin`](embedded_hal::digital::v2::OutputPin) signature.
    ChipSelectPinError,
    /// The WiFiNINA’s busy pin indicated it was not ready within the time we
    /// were waiting for it after selecting it with chip select.
    ChipSelectTimeout,
    /// There was an I/O error writing to the reset pin. Really won’t happen
    /// unless the pin is on a GPIO expander or something, but it’s a part of
    /// the [`v2::OutputPin`](embedded_hal::digital::v2::OutputPin) signature.
    ResetPinError,

    /// The WiFiNINA did not send its response to a command within the expected
    /// of time.
    ResponseTimeout,
    /// We were expecting more response parameters than the chip sent. Indicates
    /// a mismatch between this driver code and the firmware.
    MissingParam(u8),
    /// The chip sent an unexpected parameter in response to a command.
    /// Indicates a mismatch between this driver code and the firmware.
    UnexpectedParam(u8),
    /// The size of the parameter in a response is different from what we
    /// expected. Indicates a mismatch between this driver code and the
    /// firmware.
    MismatchedParamSize(usize, usize),
    /// We received an error from the chip.
    ErrorResponse,
    /// The response sent by the chip did not match the command that we sent to
    /// it.
    UnexpectedResponse(u8, u8),

    /// Returned when a connection to the Wi-Fi network does not become
    /// connected in time. Read the [`WifiStatus`](enum.WifiStatus.html)
    /// for the last status message before the timeout occurred.
    ConnectionFailed(WifiStatus),

    /// Returned when the socket connection fails to establish within 3 seconds.
    SocketConnectionFailed(SocketStatus),
    /// Returned when read or write operations are attempted on a
    /// [`ConnectedSocket`](struct.ConnectedSocket.html) that has already been
    /// closed.
    SocketClosed,
    /// Returned when the WiFiNINA chip is out of internal socket connections to
    /// give out.
    NoSocketAvailable,

    /// There was an error related to the SPI bus itself.
    SpiError(SpiError),
    /// Marker that a [`core::fmt::Error`](core::fmt::Error) occurred.
    /// Unfortunately, that error wraps any underlying error that might have
    /// been raised over the course of doing a write! to e.g. a
    /// [`ConnectedSocket`](struct.ConnectedSocket.html).
    FormatError,
}

impl<SpiError> Error<SpiError> {
    /// Convenience function for passing to [`map_err`](core::Result#map_err).
    /// (We can’t use the [`From`](core::convert::From) trait because `SpiError`
    /// is fully parameterized.)
    fn spi(err: SpiError) -> Error<SpiError> {
        Error::SpiError(err)
    }
}

impl<BE, CE, SE> From<WifiNinaChipSelectError<BE, CE>> for Error<SE> {
    fn from(err: WifiNinaChipSelectError<BE, CE>) -> Self {
        match err {
            WifiNinaChipSelectError::BusyPinError(_) => Error::ChipSelectPinError,
            WifiNinaChipSelectError::CsPinError(_) => Error::ChipSelectPinError,
            WifiNinaChipSelectError::DeviceReadyTimeout => Error::ChipSelectTimeout,
        }
    }
}

impl<SpiError> core::convert::From<core::fmt::Error> for Error<SpiError> {
    fn from(_: core::fmt::Error) -> Error<SpiError> {
        // All FormatErrors are the same (they indicate swallowing an I/O error)
        // so we don’t need to include the input argument in this output.
        Error::FormatError
    }
}
