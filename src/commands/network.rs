use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::spi::FullDuplex;

use crate::commands::*;
use crate::util::millis::Milliseconds;
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
  Spi: FullDuplex<u8, Error = SpiError>
    + embedded_hal::blocking::spi::Write<u8, Error = SpiError>
    + embedded_hal::blocking::spi::WriteIter<u8, Error = SpiError>,
  CountDown: embedded_hal::timer::CountDown<Time = CountDownTime>,
  CountDownTime: From<Milliseconds>,
{
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

  pub fn resolve_host_name(
    &mut self,
    spi: &mut Spi,
    name: &str,
  ) -> Result<[u8; 4], Error<SpiError>> {
    let mut ip = [0u8; 4];

    self.send_and_receive(
      spi,
      NinaCommand::RequestHostByName,
      Params::of(&mut [SendParam::Bytes(&mut name.bytes())]),
      Params::of(&mut [RecvParam::Ack]),
    )?;

    self.send_and_receive(
      spi,
      NinaCommand::GetHostByName,
      Params::none(),
      Params::of(&mut [RecvParam::ByteArray(&mut ip)]),
    )?;

    Ok(ip)
  }
}
