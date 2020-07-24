use cortex_m::peripheral::{syst::SystClkSource, SYST};
use embedded_hal::blocking::delay::DelayMs;
use embedded_hal::timer::CountDown;
use nb;
use nb::block;
use void::Void;
use wifinina::util::millis::{Milliseconds, U32Ext};

#[allow(non_upper_case_globals)]
static mut tick_count: Milliseconds = Milliseconds(0);

pub struct SysTickDelay {
    syst: SYST,
}

/// Wrapper around SYST that can generate SysTickCountDown objects to count down
/// arbitrary time amounts. Also implements DelayMs for doing simple blocking
/// delays.
///
/// We do this because the wifi code needs to be able to have nested timeouts of
/// various amounts.
///
/// Sets SysTick to generate an interrupt every 1ms, and increments a global
/// integer when that happens. Not guaranteed to be accurate if things are
/// pausing interrupts for longer than 1ms.
///
/// You must call SysTick::interrupt() from a "SysTick" #[exception] interrupt
/// handler in your app code.
///
/// Built on a global static value, tick_count, so that apps don’t need to
/// figure out how to make the SysTick object accessible in the interrupt
/// handler.
///
/// TODO(fiona): It might be nice to more accurately do this as a single object,
/// rather than relying on the global variable.
impl SysTickDelay {
    /// sysclock_hertz is the clock speed of the chip. You can get this from the
    /// frequency of clocks.gclk0 in a SAMD chip:
    ///
    /// let sysclock_hertz: Hertz = clocks.gclk0().into();
    pub fn new(mut syst: SYST, sysclock_hertz: u32) -> Self {
        syst.set_clock_source(SystClkSource::Core);

        syst.set_reload(sysclock_hertz / 1_000 - 1);

        syst.enable_counter();
        syst.enable_interrupt();

        SysTickDelay { syst }
    }

    #[allow(dead_code)]
    /// Use this function to recover the SYST object. Does not do any resetting
    /// of the SYST behavior.
    pub fn free(self) -> SYST {
        self.syst
    }

    /// Constructs a new CountDown instance that is backed by SysTick.
    pub fn count_down(&self) -> SysTickCountDown {
        SysTickCountDown {
            target_millis: None,
        }
    }

    /// Returns the current number of milliseconds since this whole thing was
    /// set up.
    pub fn millis() -> Milliseconds {
        unsafe { core::ptr::read_volatile(&tick_count) }
    }

    /// Call this static function from your SysTick #[exception] interrupt
    /// handler.
    pub fn interrupt() {
        // We can be unsafe because we expect that this is only called from one
        // place, an interrupt handler that only goes off every 1ms.
        unsafe {
            let cur = core::ptr::read_volatile(&tick_count);
            core::ptr::write_volatile(&mut tick_count, cur + Milliseconds(1));
        }
    }
}

impl DelayMs<u32> for SysTickDelay {
    /// Blocks until ms milliseconds have passed.
    fn delay_ms(&mut self, ms: u32) {
        let mut count_down = self.count_down();
        count_down.start(ms.ms());
        block!(count_down.wait()).unwrap();
    }
}

/// CountDown implementation powered by SysTickDelay.
///
/// TODO(fiona): Can this reference the specific SysTickDelay instance it was
/// built from?
pub struct SysTickCountDown {
    target_millis: Option<Milliseconds>,
}

impl CountDown for SysTickCountDown {
    type Time = Milliseconds;

    fn start<T>(&mut self, count: T)
    where
        T: Into<Self::Time>,
    {
        self.target_millis = Some(SysTickDelay::millis() + count.into());
    }

    fn wait(&mut self) -> Result<(), nb::Error<Void>> {
        let target_millis = self.target_millis.unwrap();

        if target_millis.is_after(SysTickDelay::millis()) {
            self.target_millis.take();
            Ok(())
        } else {
            Err(nb::Error::WouldBlock)
        }
    }
}
