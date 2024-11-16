use core::time::Duration;

use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::spi::FullDuplex;

use crate::commands::*;
use crate::{Error, WifiNina};

pub enum ArduinoPinMode {
    Input = 0,
    Output = 1,
    InputPullup = 2,

    Unknown = 255,
}

impl From<u8> for ArduinoPinMode {
    fn from(s: u8) -> Self {
        match s {
            0 => ArduinoPinMode::Input,
            1 => ArduinoPinMode::Output,
            2 => ArduinoPinMode::InputPullup,

            _ => ArduinoPinMode::Unknown,
        }
    }
}

impl Into<u8> for ArduinoPinMode {
    fn into(self) -> u8 {
        self as u8
    }
}

/// Constants for
/// [`pin_analog_read`](struct.WifiNina.html#method.pin_analog_read).
///
/// Taken from the [esp-idf
/// adc_types.h](https://github.com/espressif/esp-idf/blob/cf056a7d0b90261923b8207f21dc270313b67456/components/soc/include/hal/adc_types.h)
/// file.
pub enum EspAttenuation {
    Db0 = 0,
    Db2_5 = 1,
    Db6 = 2,
    Db11 = 3,
}

impl Into<u8> for EspAttenuation {
    fn into(self) -> u8 {
        self as u8
    }
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
    /// Puts the chip in debug mode. This causes it to write diagnostic info to
    /// its UART at 115200 baud.
    ///
    /// The debug output is fairly mundane; just echoing the command parameters
    /// response parameters, so it’s of no particular interest now that the
    /// library is set up and configured.
    pub fn set_debug(&mut self, spi: &mut Spi, enabled: bool) -> Result<(), Error<SpiError>> {
        self.send_and_receive(
            spi,
            NinaCommand::SetDebug,
            Params::of(&mut [SendParam::U8(enabled as u8)]),
            Params::of(&mut [RecvParam::Ack]),
        )
    }

    /// Returns the internal temperature of the ESP32 chip in what seems to be
    /// degrees Celsius.
    pub fn get_temperature(&mut self, spi: &mut Spi) -> Result<f32, Error<SpiError>> {
        let mut temp: f32 = 0.0;

        self.send_and_receive(
            spi,
            NinaCommand::GetTemperature,
            Params::none(),
            Params::of(&mut [RecvParam::Float(&mut temp)]),
        )?;

        Ok(temp)
    }

    /// Returns a string representation of the firmware version loaded into the
    /// ESP32, along with the length that the buffer is filled. The length will
    /// include the null character that terminates the C string. Currently the
    /// firmware version is 6 bytes long (including null character), so we
    /// allocate 10 bytes for safety.
    pub fn get_firmware_version(
        &mut self,
        spi: &mut Spi,
    ) -> Result<([u8; 10], usize), Error<SpiError>> {
        let mut buf = [0u8; 10];
        let mut size: usize = 0;

        self.send_and_receive(
            spi,
            NinaCommand::GetFirmwareVersion,
            Params::none(),
            Params::of(&mut [RecvParam::U8Buffer(&mut buf, &mut size)]),
        )?;

        Ok((buf, size))
    }

    /// Sets the mode of a pin on the ESP32.
    ///
    /// Mostly useful for the AirLift, which has pins to control an onboard RGB
    /// LED.
    pub fn pin_set_mode(
        &mut self,
        spi: &mut Spi,
        pin: u8,
        mode: ArduinoPinMode,
    ) -> Result<(), Error<SpiError>> {
        self.send_and_receive(
            spi,
            NinaCommand::SetPinMode,
            Params::of(&mut [SendParam::U8(pin), SendParam::U8(mode.into())]),
            Params::of(&mut [RecvParam::Ack]),
        )?;

        Ok(())
    }

    pub fn pin_digital_write(
        &mut self,
        spi: &mut Spi,
        pin: u8,
        value: u8,
    ) -> Result<(), Error<SpiError>> {
        self.send_and_receive(
            spi,
            NinaCommand::SetDigitalWrite,
            Params::of(&mut [SendParam::U8(pin), SendParam::U8(value)]),
            Params::of(&mut [RecvParam::Ack]),
        )?;

        Ok(())
    }

    pub fn pin_analog_write(
        &mut self,
        spi: &mut Spi,
        pin: u8,
        value: u8,
    ) -> Result<(), Error<SpiError>> {
        self.send_and_receive(
            spi,
            NinaCommand::SetAnalogWrite,
            Params::of(&mut [SendParam::U8(pin), SendParam::U8(value)]),
            Params::of(&mut [RecvParam::Ack]),
        )?;

        Ok(())
    }

    /// Reads a digital value off of one of the ESP32’s pins.
    ///
    /// Not available in Adafruit firmware before v1.5.0.
    pub fn pin_digital_read(&mut self, spi: &mut Spi, pin: u8) -> Result<u8, Error<SpiError>> {
        let mut value: u8 = 0;

        self.send_and_receive(
            spi,
            NinaCommand::SetDigitalRead,
            Params::of(&mut [SendParam::U8(pin)]),
            Params::of(&mut [RecvParam::U8(&mut value)]),
        )?;

        Ok(value)
    }

    /// Reads an analog value off of one of the ESP32’s pins.
    ///
    /// See [ESP Analog To Digital
    /// Sensor](https://esphome.io/components/sensor/adc.html) for more details.
    ///
    /// Not available in Adafruit firmware before v1.5.0.
    pub fn pin_analog_read(
        &mut self,
        spi: &mut Spi,
        pin: u8,
        attenuation: EspAttenuation,
    ) -> Result<i32, Error<SpiError>> {
        let mut value: i32 = 0;

        self.send_and_receive(
            spi,
            NinaCommand::SetAnalogRead,
            Params::of(&mut [SendParam::U8(pin), SendParam::U8(attenuation.into())]),
            Params::of(&mut [RecvParam::I32LE(&mut value)]),
        )?;

        Ok(value)
    }
}
