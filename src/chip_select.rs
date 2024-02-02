use core::time::Duration;

use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::timer::CountDown;

use crate::util::safe_spi::{ChipSelect, SafeSpi};
use crate::util::timeout_iter::IntoTimeoutIter;

#[derive(Debug)]
pub enum WifiNinaChipSelectError<CsPinError, BusyPinError> {
    CsPinError(CsPinError),
    BusyPinError(BusyPinError),
    DeviceReadyTimeout,
}

/// A ChipSelect implementation that listens to the ESP32’s “busy” output and
/// only returns selected when it’s indictating that the device is ready to
/// listen.
///`
/// Its select method needs a timer in order to fail if the device isn’t ready
/// by a deadline.
///
/// Note: does not manage exclusivity for multiple devices on the SPI bus. For a
/// PyPortal, this isn’t a problem because it has an exclusive SPI bus for the
/// Wifi co-processor. For other devices, it’s the application’s responsibility
/// to make sure that [`WifiNina`](struct.WifiNina.html) has exclusive access to
/// the bus.
pub struct WifiNinaChipSelect<S, CsPin: OutputPin, BusyPin: InputPin> {
    spi: core::marker::PhantomData<S>,

    cs: CsPin,
    busy: BusyPin,

    /// We store the last error that happened during deselection, though it’s
    /// unlikely to matter. Stored here because deselect cannot return an Err,
    /// since it’s called on SafeSpi going out-of-scope.
    last_deselect_err: Option<WifiNinaChipSelectError<CsPin::Error, BusyPin::Error>>,
}

impl<S, CsPin, BusyPin> WifiNinaChipSelect<S, CsPin, BusyPin>
where
    CsPin: OutputPin,
    BusyPin: InputPin,
{
    /// Drives the CS pin high on creation, which is “deselect.”
    pub fn new(
        mut cs: CsPin,
        busy: BusyPin,
    ) -> Result<Self, WifiNinaChipSelectError<CsPin::Error, BusyPin::Error>> {
        cs.set_high().map_err(WifiNinaChipSelectError::CsPinError)?;

        Ok(WifiNinaChipSelect {
            spi: core::marker::PhantomData,
            cs,
            busy,
            last_deselect_err: None,
        })
    }

    /// Waits 10s for the chip to not be busy, then sets the chip select pin to
    /// low to tell the chip to listen to us. Waits for 1s for the chip to
    /// acknowledge that it is selected by setting its busy pin high.
    pub fn select<'a>(
        &'a mut self,
        spi: &'a mut S,
        timer: &mut impl CountDown<Time = impl From<Duration>>,
    ) -> Result<SafeSpi<'a, S, Self>, WifiNinaChipSelectError<CsPin::Error, BusyPin::Error>> {
        // 10s value taken from CircuitPython library.
        self.wait_for_busy(timer, Duration::from_millis(10_000), false)?;

        self.cs
            .set_low()
            .map_err(WifiNinaChipSelectError::CsPinError)?;

        // We need to wait for the chip to acknowledge that it has been selected
        // before we can start sending it data.
        self.wait_for_busy(timer, Duration::from_millis(1_000), true)?;

        Ok(SafeSpi::new(spi, self))
    }

    /// Blocks until the WiFiNINA co-processor’s busy pin matches val, or
    /// returns a DeviceReadyTimeout Err if timeout milliseconds elapses.
    fn wait_for_busy(
        &mut self,
        timer: &mut impl CountDown<Time = impl From<Duration>>,
        timeout: Duration,
        val: bool,
    ) -> Result<(), WifiNinaChipSelectError<CsPin::Error, BusyPin::Error>> {
        for _ in timer.timeout_iter(timeout) {
            match self.busy.is_high() {
                Ok(b) => {
                    if b == val {
                        return Ok(());
                    }
                }
                Err(err) => return Err(WifiNinaChipSelectError::BusyPinError(err)),
            }
        }

        Err(WifiNinaChipSelectError::DeviceReadyTimeout)
    }
}

impl<S, CsPin, BusyPin> ChipSelect for WifiNinaChipSelect<S, CsPin, BusyPin>
where
    CsPin: OutputPin,
    BusyPin: InputPin,
{
    type Spi = S;

    /// To deselect the chip we drive the pin high to tell it we’re not
    /// listening anymore.
    fn deselect(&mut self) {
        self.last_deselect_err = self
            .cs
            .set_high()
            .map_err(WifiNinaChipSelectError::CsPinError)
            .err();
    }
}
