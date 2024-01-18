// Can turn off when https://github.com/rust-lang/rust/issues/8995 is fixed
#![allow(clippy::type_complexity)]

#[cfg(feature = "genio-traits")]
use genio;
#[cfg(feature = "genio-traits")]
use void;

use core::convert::TryInto;
use core::time::Duration;

use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::spi::FullDuplex;
use embedded_hal::timer::CountDown;

use crate::commands::*;
use crate::{Error, WifiNina};

/// WiFiNINA has a 4092 byte command buffer limit. See: SPI_MAX_DMA_LEN
/// https://github.com/espressif/esp-idf/blob/master/components/driver/include/driver/spi_common.h#L31
///
/// Using 4000 gives us a safety amount of bytes for the command, length, and
/// other things.
pub const MAX_WRITE_BYTES: usize = 4000;

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
    /// Allocates a socket on the WiFiNINA chip.
    ///
    /// There are a maximum of 255 available sockets at one time.
    ///
    /// We return a Socket of a different lifetime because we don’t actually
    /// enforce that the Socket value lasts as long as the references to self/spi
    /// since it doesn’t ever access them directly. They’re only there so that
    /// Sockets from one WifiNina instance can’t be used as inputs to another.
    ///
    /// Callers will typically use [`connect`](#method.connect) rather than
    /// `socket_new` directly.
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

    /// Returns the status of the given `Socket`
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

    /// Makes a network connection with the given socket.
    ///
    /// Waits 3 seconds for the connection to be established.
    pub fn socket_open<'a>(
        &'a mut self,
        spi: &'a mut Spi,
        socket: &Socket<CsPin, Spi>,
        protocol: Protocol,
        destination: Destination,
        port: u16,
    ) -> Result<SocketStatus, Error<SpiError>> {
        let mut result: Option<u8> = None;

        let ip = match destination {
            Destination::Ip(ip) => ip,
            Destination::Hostname(name) => match self.resolve_host_name(spi, name)? {
                Some(ip) => ip,
                // TODO(fiona): Should we use a different return value for a
                // host name lookup failing?
                None => return Ok(SocketStatus::Closed),
            },
        };

        self.send_and_receive(
            spi,
            NinaCommand::StartClientTcp,
            Params::of(&mut [
                SendParam::Bytes(&mut ip.iter().cloned()),
                SendParam::Word(port),
                SendParam::Byte(socket.num()),
                SendParam::Byte(protocol.into()),
            ]),
            Params::of(&mut [RecvParam::OptionalByte(&mut result)]),
        )?;

        // The WiFiNINA commands seem to indicate that it’s possible to send the
        // hostname when making a TCP connection, but when I try we get a
        // response indicating that the DNS failed.
        //
        // Both the CircuitPython and WiFiClient libraries appear to do a
        // hostname lookup ahead of time anyway, rather than trying to do it on
        // the chip, so we’re going to just move to that model.
        //
        // Unfortunately it is very unclear why this doesn’t work, as the code
        // paths for resolve_host_name and the host name version of connect seem
        // almost identical, given the C headers.
        //
        // Destination::Hostname(name) => self.send_and_receive(spi,
        //     NinaCommand::StartClientTcp, Params::of(&mut
        //     [SendParam::Bytes(&mut name.bytes()), // You still pass an IP for
        //     the host name version, but it’s // ignored. SendParam::Bytes(&mut
        //     [0, 0, 0, 0].iter().cloned()), SendParam::Word(port),
        //     SendParam::Byte(socket.num()), SendParam::Byte(protocol.into()),
        //     ]),
        //     Params::of(&mut [RecvParam::OptionalByte(&mut result)]),
        // )?,

        if result.is_none() {
            // WiFiNINA provides no return value if its internal "connect" or
            // "beginPacket" methods fail.
            return Err(Error::SocketConnectionFailed(SocketStatus::UnknownStatus));
        }

        let mut last_status = SocketStatus::UnknownStatus;

        // Wait 3 seconds for the connection.
        for _ in 0..300 {
            last_status = self.socket_status(spi, &socket)?;

            if last_status == SocketStatus::Established {
                return Ok(SocketStatus::Established);
            }

            self.timer.start(Duration::from_millis(10));
            nb::block!(self.timer.wait()).ok();
        }

        Err(Error::SocketConnectionFailed(last_status))
    }

    /// Tells the WiFiNINA chip to close the socket.
    ///
    /// Because this frees the socket num for reuse within the chip, it consumes
    /// the [`Socket`](struct.Socket.html) instance so it can’t be used again.
    pub fn socket_close(
        &mut self,
        spi: &mut Spi,
        socket: Socket<CsPin, Spi>,
    ) -> Result<(), Error<SpiError>> {
        self.send_and_receive(
            spi,
            NinaCommand::StopClientTcp,
            Params::of(&mut [SendParam::Byte(socket.num())]),
            Params::of(&mut [RecvParam::Ack]),
        )
    }

    /// Makes a network connection to the given server.
    ///
    /// Creates a new socket on the chip, opens the connection, and returns a
    /// [`ConnectedSocket`](struct.ConnectedSocket.html) to automatically close
    /// the connection.
    ///
    /// TODO(fiona): Make this work with UDP, which needs to create a server
    /// socket. [CircuitPython
    /// code](https://github.com/adafruit/Adafruit_CircuitPython_ESP32SPI/blob/522df976fd25f0ddd8648bfe5324b6e30f76d0a0/adafruit_esp32spi/adafruit_esp32spi.py#L754)
    pub fn connect<'wifi, 'sock>(
        &'wifi mut self,
        spi: &'wifi mut Spi,
        protocol: Protocol,
        destination: Destination,
        port: u16,
    ) -> Result<
        ConnectedSocket<'wifi, 'sock, CsPin, BusyPin, Spi, SpiError, CountDown, CountDownTime>,
        Error<SpiError>,
    > {
        let socket = self.socket_new(spi)?;

        self.socket_open(spi, &socket, protocol, destination, port)?;

        Ok(ConnectedSocket::new(spi, self, socket))
    }

    /// Converts a [`Socket`](struct.Socket.html) into a
    /// [`ConnectedSocket`](struct.ConnectedSocket.html).
    ///
    /// Used to get a `ConnectedSocket` back after a call to
    /// [`suspend`](struct.ConnectedSocket.html#method.suspend).
    ///
    /// Note that this is entirely a logic safety move, it doesn’t "reconnect"
    /// in any way.
    pub fn socket_resume<'wifi, 'sock>(
        &'wifi mut self,
        spi: &'wifi mut Spi,
        socket: Socket<'sock, CsPin, Spi>,
    ) -> ConnectedSocket<'wifi, 'sock, CsPin, BusyPin, Spi, SpiError, CountDown, CountDownTime>
    {
        ConnectedSocket::new(spi, self, socket)
    }

    /// Starts a socket in server mode for the given port.
    ///
    /// Provide the `multicast_ip` option if `protocol` is
    /// [`UdpMulticast`](enum.Protocol.html#variant.UdpMulticast)
    ///
    /// Once a server is created, use [`server_select`](#method.server_select)
    /// to get [`ConnectedSocket`](struct.ConnectedSocket.html)s for clients
    /// that connect.
    ///
    /// Note: The WiFiNINA firmware does not have a command to stop a server
    /// once it’s started, so this takes away from the 255 available sockets.
    pub fn server_start<'a, 'b>(
        &'a mut self,
        spi: &'a mut Spi,
        protocol: Protocol,
        port: u16,
        multicast_ip: Option<[u8; 4]>,
    ) -> Result<ServerSocket<'b, CsPin, Spi>, Error<SpiError>> {
        let socket = self.socket_new(spi)?;

        match multicast_ip {
            Some(ip) => self.send_and_receive(
                spi,
                NinaCommand::StartServerTcp,
                Params::of(&mut [
                    SendParam::Bytes(&mut ip.iter().cloned()),
                    SendParam::Word(port),
                    SendParam::Byte(socket.num()),
                    SendParam::Byte(protocol.into()),
                ]),
                Params::of(&mut [RecvParam::Ack]),
            ),
            None => self.send_and_receive(
                spi,
                NinaCommand::StartServerTcp,
                Params::of(&mut [
                    SendParam::Word(port),
                    SendParam::Byte(socket.num()),
                    SendParam::Byte(protocol.into()),
                ]),
                Params::of(&mut [RecvParam::Ack]),
            ),
        }?;

        Ok(ServerSocket::from_socket(socket))
    }

    /// Returns [`ConnectedSocket`](struct.ConnectedSocket.html) for the next
    /// client that connects to the server.
    ///
    /// Can be used with `nb::block!` to wait for a client to connect.
    pub fn server_select<'a, 'b>(
        &'a mut self,
        spi: &'a mut Spi,
        server_socket: &ServerSocket<CsPin, Spi>,
    ) -> Result<
        ConnectedSocket<'a, 'b, CsPin, BusyPin, Spi, SpiError, CountDown, CountDownTime>,
        nb::Error<Error<SpiError>>,
    > {
        // Sockets are normally 1 byte, but since the AvailableDataTcp function
        // can also return a length it responds with a full word in all cases.
        let mut socket_num: u16 = 0;

        self.send_and_receive(
            spi,
            NinaCommand::AvailableDataTcp,
            Params::of(&mut [SendParam::Byte(server_socket.num())]),
            Params::of(&mut [RecvParam::LEWord(&mut socket_num)]),
        )
        .map_err(nb::Error::Other)?;

        if socket_num == 255 {
            return Err(nb::Error::WouldBlock);
        }

        Ok(ConnectedSocket::new(
            spi,
            self,
            Socket::new(socket_num.try_into().unwrap()),
        ))
    }

    /// Writes a stream of binary data to the given client socket.
    ///
    /// Works on TCP and TLS sockets.
    ///
    /// If you want to use I/O traits, use
    /// [`ConnectedSocket`](struct.ConnectedSocket.html) instead.
    pub fn socket_write(
        &mut self,
        spi: &mut Spi,
        socket: &Socket<CsPin, Spi>,
        bytes: &mut dyn ExactSizeIterator<Item = u8>,
    ) -> Result<usize, Error<SpiError>> {
        let mut bytes_written: usize = 0;
        let mut bytes_left = bytes.len();

        // We can only write up to 4000 bytes at a time (MAX_WRITE_BYTES) so we
        // loop to write in 4000 byte chunks as necessary.
        while bytes_left > 0 {
            let mut bytes_just_written = 0u16;

            self.send_and_receive(
                spi,
                NinaCommand::SendDataTcp,
                Params::with_16_bit_length(&mut [
                    SendParam::Byte(socket.num()),
                    SendParam::Bytes(&mut bytes.take(MAX_WRITE_BYTES)),
                ]),
                // Yes, this comes back in little-endian rather than in network order.
                Params::of(&mut [RecvParam::LEWord(&mut bytes_just_written)]),
            )?;

            let bytes_just_written_usize: usize = bytes_just_written.into();
            bytes_written += bytes_just_written_usize;
            bytes_left -= bytes_just_written_usize;
        }

        Ok(bytes_written)
    }

    /// Copy of socket_write but for UDP
    ///
    /// May crash
    ///
    /// Exercise caution
    pub fn socket_write_udp(
        &mut self,
        spi: &mut Spi,
        socket: &ServerSocket<CsPin, Spi>,
        bytes: &mut dyn ExactSizeIterator<Item = u8>,
    ) -> Result<usize, Error<SpiError>> {
        let mut bytes_written: usize = 0;
        let mut bytes_left = bytes.len();

        // We can only write up to 4000 bytes at a time (MAX_WRITE_BYTES) so we
        // loop to write in 4000 byte chunks as necessary.
        while bytes_left > 0 {
            let mut bytes_just_written = 0u16;

            self.send_and_receive(
                spi,
                NinaCommand::InsertDatabuf,
                Params::with_16_bit_length(&mut [
                    SendParam::Byte(socket.num()),
                    SendParam::Bytes(&mut bytes.take(MAX_WRITE_BYTES)),
                ]),
                // Yes, this comes back in little-endian rather than in network order.
                Params::of(&mut [RecvParam::LEWord(&mut bytes_just_written)]),
            )?;

            let bytes_just_written_usize: usize = bytes_just_written.into();
            bytes_written += bytes_just_written_usize;
            bytes_left -= bytes_just_written_usize;
        }

        Ok(bytes_written)
    }

    /// Sends data prepared with [WifiNina::socket_write_udp()](#method.socket_write_udp), purging the buffer
    ///
    /// Data on the UDP socket will accumulate until sent.
    ///
    /// You can only get rid of it by sending (?)
    pub fn socket_send_udp(
        &mut self,
        spi: &mut Spi,
        socket: &ServerSocket<CsPin, Spi>,
    ) -> Result<usize, Error<SpiError>> {
        let mut response = 0u16;

        self.send_and_receive(
            spi,
            NinaCommand::SendUdpData,
            Params::of(&mut [SendParam::Byte(socket.num())]),
            Params::of(&mut [RecvParam::LEWord(&mut response)]),
        )?;

        let response_usize: usize = response.into();

        Ok(response_usize) // TODO: Find out what the command should return
    }

    /// Reads binary data from a socket into the buffer.
    ///
    /// Can read TCP, TLS, and UDP data. (Not UDP multicast.)
    ///
    /// Returns the number of bytes read, or 0 if the socket has been closed.
    ///
    /// Returns [`nb::Error::WouldBlock`](nb::Error::WouldBlock) if there’s no
    /// data for the client.
    ///
    /// If you want to use I/O traits, use
    /// [`ConnectedSocket`](struct.ConnectedSocket.html) instead.
    pub fn socket_read(
        &mut self,
        spi: &mut Spi,
        socket: &Socket<CsPin, Spi>,
        buf: &mut [u8],
    ) -> Result<usize, nb::Error<Error<SpiError>>> {
        let mut available: u16 = 0;

        // When given a client socket, AvailableDataTcp returns the number of
        // bytes available.
        //
        // TODO(fiona): Do we need to check for available data before reading?
        // Could we just return WouldBlock if the read result is 0?
        self.send_and_receive(
            spi,
            NinaCommand::AvailableDataTcp,
            Params::of(&mut [SendParam::Byte(socket.num())]),
            Params::of(&mut [RecvParam::LEWord(&mut available)]),
        )
        .map_err(nb::Error::Other)?;

        if available == 0 {
            return match self.socket_status(spi, socket)? {
                SocketStatus::Closed => Ok(0),
                _ => Err(nb::Error::WouldBlock),
            };
        }

        let read_limit = core::cmp::min(available, buf.len() as u16);
        let mut read_len: usize = 0;

        self.send_and_receive(
            spi,
            NinaCommand::GetDatabufTcp,
            Params::with_16_bit_length(&mut [
                SendParam::Byte(socket.num()),
                SendParam::LEWord(read_limit),
            ]),
            Params::with_16_bit_length(&mut [RecvParam::Buffer(buf, &mut read_len)]),
        )
        .map_err(nb::Error::Other)?;

        Ok(read_len)
    }
}

/// Numeric reference to a socket held on the WiFiNINA ESP32 chip.
///
/// We include the Spi and the chip select in the type so that `Socket`s can’t
/// be re-used across [`WifiNina`](struct.WifiNina.html) instances.
///
/// Instances are not copyable because they represent specific resources on the
/// chip.
///
/// TODO(fiona): Should we have different types for TCP/TLS, UDP, and Multicast?
pub struct Socket<'a, CS, S> {
    // These are refs because the docs for [`PhantomData`] say to use refs when
    // there’s not ownership.
    cs: core::marker::PhantomData<&'a CS>,
    spi: core::marker::PhantomData<&'a S>,
    num: u8,
}

impl<'a, CS, S> Socket<'a, CS, S> {
    fn new(num: u8) -> Self {
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

/// Marker for a server socket rather than a client one.
///
/// You don’t read or write directly to a `ServerSocket`, but instead call
/// [`server_select`](struct.WifiNina.html#method.server_select) to get a new
/// [`ConnectedSocket`](struct.ConnectedSocket.html) when a client connects.
///
/// The WifiNINA protocol has no means for closing a server socket once it has
/// been created.
#[derive(Copy, Clone)]
pub struct ServerSocket<'a, CS, S> {
    cs: core::marker::PhantomData<&'a CS>,
    spi: core::marker::PhantomData<&'a S>,
    num: u8,
}

impl<'a, CS, S> ServerSocket<'a, CS, S> {
    pub fn from_socket(s: Socket<CS, S>) -> Self {
        ServerSocket {
            cs: core::marker::PhantomData,
            spi: core::marker::PhantomData,
            num: s.num,
        }
    }

    pub fn num(&self) -> u8 {
        self.num
    }
}

impl<'a, CS, S> core::fmt::Debug for ServerSocket<'a, CS, S> {
    fn fmt(
        &self,
        fmt: &mut core::fmt::Formatter<'_>,
    ) -> core::result::Result<(), core::fmt::Error> {
        write!(fmt, "ServerSocket[{}]", self.num)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(u8)]
pub enum Protocol {
    Tcp = 0,
    Udp = 1,
    Tls = 2,
    UdpMulticast = 3,
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

/// Wrapper around [`Socket`](struct.Socket.html) to provide I/O traits and to
/// automatically close it on drop.
///
/// Because the WiFiNINA chip has a limited number of sockets allocated, it’s
/// important to close them when they’re done.
///
/// This object mutably borrows both the `Spi` and the
/// [`WifiNina`](struct.WifiNina.html) value, so you can’t use your
/// application’s `WifiNina` for anything else while a `ConnectedSocket` is in
/// scope. If you need to, you can call [`suspend`](#method.suspend) to return
/// the inner [`Socket`](struct.Socket.html) value and release the mutable
/// references. Use `WifiNina`’s
/// [`socket_resume`](struct.WifiNina.html#socket_resume) method to turn that
/// `Socket` back into a `ConnectedSocket`.
///
/// If you `suspend` but don’t call `socket_resume`, make sure that you call
/// [`socket_close`](struct.WifiNina.html#method.socket_close) on the `Socket`
/// to avoid leaking resources on the chip.
///
/// TODO(fiona): Figure out if there’s a nicer way to type this.
pub struct ConnectedSocket<'wifi, 'sock, CS, B, S, SE, T, TC>
where
    CS: OutputPin,
    B: InputPin,
    SE: core::fmt::Debug,
    S: FullDuplex<u8, Error = SE>
        + embedded_hal::blocking::spi::Write<u8, Error = SE>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SE>,
    T: CountDown<Time = TC>,
    TC: From<Duration>,
{
    spi: &'wifi mut S,
    wifi: &'wifi mut WifiNina<CS, B, S, T>,
    // Socket has a separate lifetime so it can exist outside of the mutable
    // borrows of spi and wifi.
    socket: Option<Socket<'sock, CS, S>>,
}

impl<'wifi, 'sock, CS, B, S, SE, T, TC> ConnectedSocket<'wifi, 'sock, CS, B, S, SE, T, TC>
where
    CS: OutputPin,
    B: InputPin,
    SE: core::fmt::Debug,
    S: FullDuplex<u8, Error = SE>
        + embedded_hal::blocking::spi::Write<u8, Error = SE>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SE>,
    T: CountDown<Time = TC>,
    TC: From<Duration>,
{
    pub fn new(
        spi: &'wifi mut S,
        wifi: &'wifi mut WifiNina<CS, B, S, T>,
        socket: Socket<'sock, CS, S>,
    ) -> Self {
        ConnectedSocket {
            spi,
            wifi,
            socket: Some(socket),
        }
    }

    /// Reads binary data from the socket into the buffer.
    ///
    /// Can read TCP, TLS, and UDP data. (Not UDP multicast.)
    ///
    /// Returns the number of bytes read, or 0 if the socket has been closed.
    ///
    /// Returns [`nb::Error::WouldBlock`](nb::Error::WouldBlock) if there’s no
    /// data for the client.
    ///
    /// See: [`socket_read`](struct.WifiNina.html#method.socket_read)
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, nb::Error<Error<SE>>> {
        let socket = self
            .socket
            .as_ref()
            .ok_or(nb::Error::Other(Error::SocketClosed))?;

        self.wifi.socket_read(self.spi, socket, buf)
    }

    /// Writes a stream of binary data to the socket.
    ///
    /// Works on TCP and TLS sockets.
    ///
    /// See: [`socket_write`](struct.WifiNina.html#method.socket_write)
    pub fn write(&mut self, buf: &[u8]) -> Result<usize, Error<SE>> {
        let socket = self.socket.as_ref().ok_or(Error::SocketClosed)?;

        self.wifi
            .socket_write(self.spi, socket, &mut buf.iter().cloned())
    }

    /// Returns the underlying [`Socket`](struct.Socket.html) value without
    /// closing it.
    ///
    /// Use this when you need to free up the `WifiNina` and `Spi` values for
    /// other operations while keeping the socket open.
    pub fn suspend(mut self) -> Socket<'sock, CS, S> {
        self.socket.take().unwrap()
    }

    /// Closes this socket explicitly, releasing the resources on the chip.
    ///
    /// This will happen on Drop if it’s not done manually.
    ///
    /// See: [`socket_close`](struct.WifiNina.html#method.socket_close)
    pub fn close(mut self) -> Result<(), Error<SE>> {
        match self.socket.take() {
            Some(socket) => self.wifi.socket_close(self.spi, socket),
            None => Err(Error::SocketClosed),
        }
    }
}

impl<'wifi, 'sock, CS, B, S, SE, T, TC> Drop for ConnectedSocket<'wifi, 'sock, CS, B, S, SE, T, TC>
where
    CS: OutputPin,
    B: InputPin,
    SE: core::fmt::Debug,
    S: FullDuplex<u8, Error = SE>
        + embedded_hal::blocking::spi::Write<u8, Error = SE>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SE>,
    T: CountDown<Time = TC>,
    TC: From<Duration>,
{
    /// Closes the socket, if it hasn’t been.
    ///
    /// See: [`socket_close`](struct.WifiNina.html#method.socket_close)
    fn drop(&mut self) {
        if let Some(socket) = self.socket.take() {
            self.wifi.socket_close(self.spi, socket).ok();
        }
    }
}

/// Lets you use `write!` to write to a `ConnectedSocket`.
impl<'wifi, 'sock, CS, B, S, SE, T, TC> core::fmt::Write
    for ConnectedSocket<'wifi, 'sock, CS, B, S, SE, T, TC>
where
    CS: OutputPin,
    B: InputPin,
    SE: core::fmt::Debug,
    S: FullDuplex<u8, Error = SE>
        + embedded_hal::blocking::spi::Write<u8, Error = SE>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SE>,
    T: CountDown<Time = TC>,
    TC: From<Duration>,
{
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        match self.write(s.as_bytes()) {
            Ok(_) => Ok(()),
            Err(_) => Err(core::fmt::Error),
        }
    }
}

#[cfg(feature = "genio-traits")]
impl<'wifi, 'sock, CS, B, S, SE, T, TC> genio::Read
    for ConnectedSocket<'wifi, 'sock, CS, B, S, SE, T, TC>
where
    CS: OutputPin,
    B: InputPin,
    SE: core::fmt::Debug,
    S: FullDuplex<u8, Error = SE>
        + embedded_hal::blocking::spi::Write<u8, Error = SE>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SE>,
    T: CountDown<Time = TC>,
    TC: From<Duration>,
{
    type ReadError = nb::Error<Error<SE>>;

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::ReadError> {
        self.read(buf)
    }
}

#[cfg(feature = "genio-traits")]
impl<'wifi, 'sock, CS, B, S, SE, T, TC> genio::Write
    for ConnectedSocket<'wifi, 'sock, CS, B, S, SE, T, TC>
where
    CS: OutputPin,
    B: InputPin,
    SE: core::fmt::Debug,
    S: FullDuplex<u8, Error = SE>
        + embedded_hal::blocking::spi::Write<u8, Error = SE>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SE>,
    T: CountDown<Time = TC>,
    TC: From<Duration>,
{
    type WriteError = Error<SE>;
    type FlushError = void::Void;

    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::WriteError> {
        self.write(buf)
    }

    /// No effect.
    ///
    /// The writing operation sends data directly to the ESP32 chip, so there’s
    /// no extra flushing needed.
    fn flush(&mut self) -> Result<(), Self::FlushError> {
        Ok(())
    }

    fn size_hint(&mut self, _: usize) {}

    fn uses_size_hint(&self) -> bool {
        false
    }
}
