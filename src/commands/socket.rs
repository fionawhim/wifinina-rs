#[cfg(feature = "genio-traits")]
use genio;
#[cfg(feature = "genio-traits")]
use void;

use nb;

use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::spi::FullDuplex;
use embedded_hal::timer::CountDown;

use crate::commands::*;
use crate::util::millis::{Milliseconds, U32Ext};
use crate::{Error, WifiNina};

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
    /// Allocates a socket on the WiFiNINA chip. There are a maximum of 255
    /// available sockets.
    ///
    // We return a Socket of a different lifetime because we don’t actually
    // enforce that the Socket value lasts as long as the references to self/spi
    // since it doesn’t ever access them directly.
    pub fn socket_new<'a, 'b>(
        &'a mut self,
        spi: &'a mut Spi,
    ) -> Result<Socket<'b, CsPin, Spi>, Error<SpiError>> {
        let mut socket_num = 255u8;

        self.send_and_receive(
            spi,
            NinaCommand::GetSocket,
            Params::none(),
            Params::of(&mut [RecvParam::Byte(&mut socket_num)]),
        )?;

        if socket_num == 255 {
            return Err(Error::NoSocketAvailable);
        }

        Ok(Socket::new(socket_num))
    }

    pub fn socket_status(
        &mut self,
        spi: &mut Spi,
        socket: &Socket<CsPin, Spi>,
    ) -> Result<SocketStatus, Error<SpiError>> {
        let mut status: u8 = 255;

        self.send_and_receive(
            spi,
            NinaCommand::GetClientStateTcp,
            Params::of(&mut [SendParam::Byte(socket.num())]),
            Params::of(&mut [RecvParam::Byte(&mut status)]),
        )?;

        Ok(status.into())
    }

    pub fn socket_open<'a>(
        &'a mut self,
        spi: &'a mut Spi,
        socket: &Socket<CsPin, Spi>,
        protocol: Protocol,
        destination: Destination,
        port: u16,
    ) -> Result<SocketStatus, Error<SpiError>> {
        let mut result: Option<u8> = None;

        match destination {
            Destination::Ip(ip) => self.send_and_receive(
                spi,
                NinaCommand::StartClientTcp,
                Params::of(&mut [
                    SendParam::Bytes(&mut ip.iter().cloned()),
                    SendParam::Word(port),
                    SendParam::Byte(socket.num()),
                    SendParam::Byte(protocol.into()),
                ]),
                Params::of(&mut [RecvParam::OptionalByte(&mut result)]),
            )?,
            Destination::Hostname(name) => self.send_and_receive(
                spi,
                NinaCommand::StartClientTcp,
                Params::of(&mut [
                    SendParam::Bytes(&mut name.bytes()),
                    SendParam::Bytes(&mut [0, 0, 0, 0].iter().cloned()),
                    SendParam::Word(port),
                    SendParam::Byte(socket.num()),
                    SendParam::Byte(protocol.into()),
                ]),
                Params::of(&mut [RecvParam::OptionalByte(&mut result)]),
            )?,
        }

        if let None = result {
            return Err(Error::SocketConnectionFailed(SocketStatus::UnknownStatus));
        }

        let mut last_status = SocketStatus::UnknownStatus;

        // Wait 3 seconds for the connection.
        for _ in 0..300 {
            last_status = self.socket_status(spi, &socket)?;

            if last_status == SocketStatus::Established {
                return Ok(SocketStatus::Established);
            }

            self.timer.start(10.ms());
            nb::block!(self.timer.wait()).ok();
        }

        Err(Error::SocketConnectionFailed(last_status))
    }

    pub fn socket_close(
        &mut self,
        spi: &mut Spi,
        socket: &Socket<CsPin, Spi>,
    ) -> Result<(), Error<SpiError>> {
        self.send_and_receive(
            spi,
            NinaCommand::StopClientTcp,
            Params::of(&mut [SendParam::Byte(socket.num())]),
            Params::of(&mut [RecvParam::Ack]),
        )
    }

    pub fn connect<'a>(
        &'a mut self,
        spi: &'a mut Spi,
        protocol: Protocol,
        destination: Destination,
        port: u16,
    ) -> Result<
        ConnectedSocket<'a, CsPin, BusyPin, Spi, SpiError, CountDown, CountDownTime>,
        Error<SpiError>,
    > {
        let socket = self.socket_new(spi)?;

        self.socket_open(spi, &socket, protocol, destination, port)?;

        Ok(ConnectedSocket::new(spi, self, socket))
    }

    pub fn socket_write(
        &mut self,
        spi: &mut Spi,
        socket: &Socket<CsPin, Spi>,
        bytes: &mut dyn ExactSizeIterator<Item = u8>,
    ) -> Result<usize, Error<SpiError>> {
        let mut written = 0u16;

        self.send_and_receive(
            spi,
            NinaCommand::SendDataTcp,
            Params::with_16_bit_length(&mut [
                SendParam::Byte(socket.num()),
                SendParam::Bytes(bytes),
            ]),
            // Yes, this comes back in little-endian rather than in network order.
            Params::of(&mut [RecvParam::LEWord(&mut written)]),
        )?;

        Ok(written as usize)
    }

    pub fn socket_read(
        &mut self,
        spi: &mut Spi,
        socket: &Socket<CsPin, Spi>,
        buf: &mut [u8],
    ) -> Result<usize, nb::Error<Error<SpiError>>> {
        let mut available: u16 = 0;

        self.send_and_receive(
            spi,
            NinaCommand::AvailableDataTcp,
            Params::of(&mut [SendParam::Byte(socket.num())]),
            Params::of(&mut [RecvParam::LEWord(&mut available)]),
        )
        .map_err(|err| nb::Error::Other(err))?;

        if available == 0 {
            return match self.socket_status(spi, socket)? {
                SocketStatus::Closed => Ok(0),
                _ => Err(nb::Error::WouldBlock),
            };
        }

        let req_size = core::cmp::min(available, buf.len() as u16);

        let mut read: usize = 0;

        self.send_and_receive(
            spi,
            NinaCommand::GetDatabufTcp,
            Params::with_16_bit_length(&mut [
                SendParam::Byte(socket.num()),
                SendParam::LEWord(req_size),
            ]),
            Params::with_16_bit_length(&mut [RecvParam::Buffer(buf, &mut read)]),
        )
        .map_err(|err| {
            return nb::Error::Other(err);
        })?;

        Ok(read)
    }
}

// We include the Spi and the chip select in the type as a way to keep Sockets
// from being re-used across WifiNina instances.
//
// These are refs because the docs for PhantomData say to use refs when there’s
// not ownership.
#[derive(Copy, Clone)]
pub struct Socket<'a, CS, S> {
    cs: core::marker::PhantomData<&'a CS>,
    spi: core::marker::PhantomData<&'a S>,
    num: u8,
}

impl<'a, CS, S> Socket<'a, CS, S> {
    pub fn new(num: u8) -> Self {
        Socket {
            cs: core::marker::PhantomData,
            spi: core::marker::PhantomData,
            num,
        }
    }

    pub fn num(&self) -> u8 {
        self.num
    }
}

impl<'a, CS, S> core::fmt::Debug for Socket<'a, CS, S> {
    fn fmt(
        &self,
        fmt: &mut core::fmt::Formatter<'_>,
    ) -> core::result::Result<(), core::fmt::Error> {
        write!(fmt, "Socket[{}]", self.num)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(u8)]
pub enum Protocol {
    TCP = 0,
    UDP = 1,
    TLS = 2,
}
impl Into<u8> for Protocol {
    fn into(self) -> u8 {
        self as u8
    }
}

pub enum Destination<'a> {
    Ip([u8; 4]),
    Hostname(&'a str),
}

#[repr(u8)]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SocketStatus {
    Closed = 0,
    Listen = 1,
    SynSent = 2,
    SynReceived = 3,
    Established = 4,
    FinWait1 = 5,
    FinWait2 = 6,
    CloseWait = 7,
    Closing = 8,
    LastAck = 9,
    TimeWait = 10,

    UnknownStatus = 255,
}

impl From<u8> for SocketStatus {
    fn from(s: u8) -> Self {
        match s {
            0 => SocketStatus::Closed,
            1 => SocketStatus::Listen,
            2 => SocketStatus::SynSent,
            3 => SocketStatus::SynReceived,
            4 => SocketStatus::Established,
            5 => SocketStatus::FinWait1,
            6 => SocketStatus::FinWait2,
            7 => SocketStatus::CloseWait,
            8 => SocketStatus::Closing,
            9 => SocketStatus::LastAck,
            10 => SocketStatus::TimeWait,

            _ => SocketStatus::UnknownStatus,
        }
    }
}

pub struct ConnectedSocket<'a, CS, B, S, SE, T, TC>
where
    CS: OutputPin,
    B: InputPin,
    S: FullDuplex<u8, Error = SE>
        + embedded_hal::blocking::spi::Write<u8, Error = SE>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SE>,
    T: CountDown<Time = TC>,
    TC: From<Milliseconds>,
{
    spi: &'a mut S,
    wifi: &'a mut WifiNina<CS, B, S, T>,
    // TODO(fiona): Make this an Option so it can be consumed on close.
    socket: Socket<'a, CS, S>,
}

impl<'a, CS, B, S, SE, T, TC> ConnectedSocket<'a, CS, B, S, SE, T, TC>
where
    CS: OutputPin,
    B: InputPin,
    S: FullDuplex<u8, Error = SE>
        + embedded_hal::blocking::spi::Write<u8, Error = SE>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SE>,
    T: CountDown<Time = TC>,
    TC: From<Milliseconds>,
{
    pub fn new(
        spi: &'a mut S,
        wifi: &'a mut WifiNina<CS, B, S, T>,
        socket: Socket<'a, CS, S>,
    ) -> Self {
        ConnectedSocket { spi, wifi, socket }
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, nb::Error<Error<SE>>> {
        self.wifi.socket_read(self.spi, &self.socket, buf)
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize, Error<SE>> {
        self.wifi
            .socket_write(self.spi, &self.socket, &mut buf.iter().cloned())
    }
}

impl<'a, CS, B, S, SE, T, TC> Drop for ConnectedSocket<'a, CS, B, S, SE, T, TC>
where
    CS: OutputPin,
    B: InputPin,
    S: FullDuplex<u8, Error = SE>
        + embedded_hal::blocking::spi::Write<u8, Error = SE>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SE>,
    T: CountDown<Time = TC>,
    TC: From<Milliseconds>,
{
    fn drop(&mut self) {
        self.wifi.socket_close(self.spi, &self.socket).ok();
    }
}

impl<'a, CS, B, S, SE, T, TC> core::fmt::Write for ConnectedSocket<'a, CS, B, S, SE, T, TC>
where
    CS: OutputPin,
    B: InputPin,
    S: FullDuplex<u8, Error = SE>
        + embedded_hal::blocking::spi::Write<u8, Error = SE>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SE>,
    T: CountDown<Time = TC>,
    TC: From<Milliseconds>,
{
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        match self.write(s.as_bytes()) {
            Ok(_) => Ok(()),
            Err(_) => Err(core::fmt::Error),
        }
    }
}

#[cfg(feature = "genio-traits")]
impl<'a, CS, B, S, SE, T, TC> genio::Read for ConnectedSocket<'a, CS, B, S, SE, T, TC>
where
    CS: OutputPin,
    B: InputPin,
    S: FullDuplex<u8, Error = SE>
        + embedded_hal::blocking::spi::Write<u8, Error = SE>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SE>,
    T: CountDown<Time = TC>,
    TC: From<Milliseconds>,
{
    type ReadError = nb::Error<Error<SE>>;

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::ReadError> {
        self.read(buf)
    }
}

#[cfg(feature = "genio-traits")]
impl<'a, CS, B, S, SE, T, TC> genio::Write for ConnectedSocket<'a, CS, B, S, SE, T, TC>
where
    CS: OutputPin,
    B: InputPin,
    S: FullDuplex<u8, Error = SE>
        + embedded_hal::blocking::spi::Write<u8, Error = SE>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SE>,
    T: CountDown<Time = TC>,
    TC: From<Milliseconds>,
{
    type WriteError = Error<SE>;
    type FlushError = void::Void;

    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::WriteError> {
        self.write(buf)
    }

    fn flush(&mut self) -> Result<(), Self::FlushError> {
        Ok(())
    }

    fn size_hint(&mut self, _: usize) {}

    fn uses_size_hint(&self) -> bool {
        false
    }
}
