use core::time::Duration;

use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::spi::FullDuplex;

use crate::commands::*;
use crate::{Error, WifiNina};

#[derive(Debug, Default)]
pub struct NetworkInfo {
    pub ip: [u8; 4],
    pub netmask: [u8; 4],
    pub gateway_ip: [u8; 4],
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
    /// Queries the chip for its IP, netmask, and gateway.
    pub fn network_info(&mut self, spi: &mut Spi) -> Result<NetworkInfo, Error<SpiError>> {
        let mut network_info: NetworkInfo = Default::default();

        self.send_and_receive(
            spi,
            NinaCommand::GetIpAddress,
            Params::none(),
            Params::of(&mut [
                RecvParam::ByteArray(&mut network_info.ip),
                RecvParam::ByteArray(&mut network_info.netmask),
                RecvParam::ByteArray(&mut network_info.gateway_ip),
            ]),
        )?;

        Ok(network_info)
    }

    /// Queries the chip to do a DNS lookup of the given hostname and return the
    /// IP address.
    ///
    /// Returns None if the DNS did not find an IP.
    pub fn resolve_host_name(
        &mut self,
        spi: &mut Spi,
        name: &str,
    ) -> Result<Option<[u8; 4]>, Error<SpiError>> {
        let mut ip = [0u8; 4];

        let mut success = 0u8;

        // The API divides this across two calls so that it can return the
        // success / failure in the first and the actual bytes in the second.
        self.send_and_receive(
            spi,
            NinaCommand::RequestHostByName,
            Params::of(&mut [SendParam::Bytes(&mut name.bytes())]),
            Params::of(&mut [RecvParam::Byte(&mut success)]),
        )?;

        if success == 0 {
            return Ok(None);
        }

        self.send_and_receive(
            spi,
            NinaCommand::GetHostByName,
            Params::none(),
            Params::of(&mut [RecvParam::ByteArray(&mut ip)]),
        )?;

        Ok(Some(ip))
    }

    /// Pings the given IP address and returns the time in ms.
    ///
    /// Note that the resolution of the ESP32 seems to be in multiples of 10.
    pub fn ping(&mut self, spi: &mut Spi, ip: &[u8; 4], ttl: u8) -> Result<u16, Error<SpiError>> {
        let mut result = 0u16;

        self.send_and_receive(
            spi,
            NinaCommand::Ping,
            Params::of(&mut [
                SendParam::Bytes(&mut ip.iter().cloned()),
                SendParam::Byte(ttl),
            ]),
            Params::of(&mut [RecvParam::LEWord(&mut result)]),
        )?;

        Ok(result)
    }
}
