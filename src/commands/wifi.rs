use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::spi::FullDuplex;

use nb::block;

use crate::commands::*;
use crate::util::millis::{Milliseconds, U32Ext};
use crate::{Error, WifiNina};

#[repr(u8)]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum WifiStatus {
    Idle = 0,
    NoSsidAvailable = 1,
    ScanCompleted = 2,
    Connected = 3,
    ConnectFailed = 4,
    ConnectionLost = 5,
    Disconnected = 6,
    ApListening = 7,
    ApConnected = 8,
    ApFailed = 9,

    UnknownStatus = 255,
}

impl From<u8> for WifiStatus {
    fn from(s: u8) -> Self {
        match s {
            0 => WifiStatus::Idle,
            1 => WifiStatus::NoSsidAvailable,
            2 => WifiStatus::ScanCompleted,
            3 => WifiStatus::Connected,
            4 => WifiStatus::ConnectFailed,
            5 => WifiStatus::ConnectionLost,
            6 => WifiStatus::Disconnected,
            7 => WifiStatus::ApListening,
            8 => WifiStatus::ApConnected,
            9 => WifiStatus::ApFailed,

            _ => WifiStatus::UnknownStatus,
        }
    }
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
    pub fn wifi_status(&mut self, spi: &mut Spi) -> Result<WifiStatus, Error<SpiError>> {
        let mut status: u8 = 255;

        self.send_and_receive(
            spi,
            NinaCommand::GetConnectionStatus,
            Params::none(),
            Params::of(&mut [RecvParam::Byte(&mut status)]),
        )?;

        Ok(status.into())
    }

    pub fn wifi_connect(
        &mut self,
        spi: &mut Spi,
        ssid: &str,
        password: Option<&str>,
    ) -> Result<WifiStatus, Error<SpiError>> {
        match password {
            None => {
                self.send_and_receive(
                    spi,
                    NinaCommand::SetNetwork,
                    Params::of(&mut [SendParam::Bytes(&mut ssid.bytes())]),
                    Params::of(&mut [RecvParam::Ack]),
                )?;
            }

            Some(password) => {
                self.send_and_receive(
                    spi,
                    NinaCommand::SetNetworkAndPassphrase,
                    Params::of(&mut [
                        SendParam::Bytes(&mut ssid.bytes()),
                        SendParam::Bytes(&mut password.bytes()),
                    ]),
                    Params::of(&mut [RecvParam::Ack]),
                )?;
            }
        }

        let mut last_status = WifiStatus::UnknownStatus;

        // Wait 10 seconds for the Wifi to stabilize.
        for _ in 0..10 {
            last_status = self.wifi_status(spi)?;

            if last_status == WifiStatus::Connected {
                return Ok(last_status);
            }

            self.timer.start(1_000.ms());
            block!(self.timer.wait()).ok();
        }

        Err(Error::ConnectionFailed(last_status))
    }

    pub fn wifi_create_ap(
        &mut self,
        spi: &mut Spi,
        name: &str,
        channel: u8,
    ) -> Result<(), Error<SpiError>> {
        self.send_and_receive(
            spi,
            NinaCommand::SetApNetwork,
            Params::of(&mut [
                SendParam::Bytes(&mut name.bytes()),
                SendParam::Byte(channel),
            ]),
            Params::of(&mut [RecvParam::Ack]),
        )
    }
}
