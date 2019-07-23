#![no_std]

mod chip_select;
mod util;
pub mod commands;

use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::spi::FullDuplex;

use nb::block;

use util::millis::{Milliseconds, U32Ext};

use chip_select::*;

use commands::{socket::SocketStatus, wifi::WifiStatus,};


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
    Spi: FullDuplex<u8, Error = SpiError>
        + embedded_hal::blocking::spi::Write<u8, Error = SpiError>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SpiError>,
    CountDown: embedded_hal::timer::CountDown<Time = CountDownTime>,
    CountDownTime: From<Milliseconds>,
{
    // const ConnectionDelayMs: u16 = 100;

    // We take the spi here just to allow the type to be implied.
    //
    // Also resets the WifiNINA chip.
    pub fn new<ResetPin>(
        _spi: &Spi,
        cs: CsPin,
        busy: BusyPin,
        reset: &mut ResetPin,
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

        wifi.reset(reset)?;

        Ok(wifi)
    }

    pub fn reset<ResetPin>(&mut self, reset: &mut ResetPin) -> Result<(), Error<SpiError>>
    where
        ResetPin: OutputPin,
    {
        reset.set_low().map_err(|_| Error::ResetPinError)?;

        self.timer.start(200.ms());
        block!(self.timer.wait()).unwrap();

        reset.set_high().map_err(|_| Error::ResetPinError)?;

        self.timer.start(750.ms());
        block!(self.timer.wait()).unwrap();

        Ok(())
    }

    // Static method because it needs to be called while device_selector is borrowed
}

#[derive(Debug)]
pub enum Error<SpiError> {
    ChipSelectPinError,
    ChipSelectTimeout,

    ResponseTimeout,
    MissingParam(u8),
    UnexpectedParam(u8),
    MismatchedParamSize(usize, usize),
    ErrorResponse,
    UnexpectedResponse(u8, u8),

    ConnectionFailed(WifiStatus),
    ConnectionTimeout,

    SocketConnectionFailed(SocketStatus),
    SocketClosed,
    SocketTimeout,
    NoSocketAvailable,

    SpiError(SpiError),
    ResetPinError,
}

impl<SpiError> Error<SpiError> {
    // Convenience function for passing to map_err, because we can’t use
    // the From trait because SpiError is fully parameterized.
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

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
