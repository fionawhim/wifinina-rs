pub mod extras;
pub mod network;
pub mod socket;
pub mod wifi;

use core::cmp::min;
use core::time::Duration;

use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::spi::FullDuplex;

use crate::util::spi_ext::SpiExt;
use crate::util::timeout_iter::IntoTimeoutIter;

use crate::{Error, WifiNina};

#[derive(Debug, Copy, Clone)]
#[repr(u8)]
/// All of the commands found in the WiFiNINA firmware. Not all are currently
/// supported by this library.
///
/// **References**
/// * [Adafruit firmware](https://github.com/adafruit/nina-fw/blob/master/main/CommandHandler.cpp)
/// * [v1.2.1 of the Adafruit
///   firmware](https://github.com/adafruit/nina-fw/tree/a55c66afb48428d5b29f89d74a7a20c587560ec7)
///   found in the Adabox PyPortal and on a recent AirLift FeatherWing
/// * [CircuitPython lib](https://github.com/adafruit/Adafruit_CircuitPython_ESP32SPI/)
pub enum NinaCommand {
    /// Joins a WiFi network without a password.
    SetNetwork = 0x10,
    /// Joins a WiFi network with a password.
    SetNetworkAndPassphrase = 0x11,
    /// Same as SetNetworkAndPassphrase, but with WEP-sized buffer for the
    /// password. Unused.
    #[allow(dead_code)]
    SetKey = 0x12,
    // Unused = 0x13,
    // SetIpConfig = 0x14,
    // SetDnsConfig = 0x15,
    // SetHostname = 0x16,
    // SetPowerMode = 0x17,
    /// Creates an access point.
    SetApNetwork = 0x18,
    /// Creates an access point with a password.
    SetApPassphrase = 0x19,
    /// Enables/disables debug logging to the ESP32’s UART
    SetDebug = 0x1A,
    /// Returns the chip’s temperature in Celsius
    GetTemperature = 0x1B,
    // Unused = 0x1C,
    // Unused = 0x1D,
    // Unused = 0x1E,
    // Unused = 0x1F,
    /// Returns a status byte about the WiFi connection.
    ///
    /// See [`WifiStatus`](enum.WifiStatus.html).
    GetConnectionStatus = 0x20,
    GetIpAddress = 0x21,
    // GetMacAddress = 0x22,
    // GetCurrentSsid = 0x23,
    // GetCurrentBssid = 0x24,
    // GetCurrentRssi = 0x25,
    // GetCurrentEnct = 0x26,
    /// Returns SSIDs for all networks the chip knows about.
    ScanNetworks = 0x27,
    /// Starts a TCP, UDP, or Multicast UDP server on a port for the provided
    /// socket.
    ///
    /// Future calls to AvailableDataTcp with that socket num will return a new
    /// socket to communicate with a client, if one is waiting.
    StartServerTcp = 0x28,
    /// Returns 1 if the socket is for a TCP server, 0 otherwise. Unused.
    #[allow(dead_code)]
    GetStateTcp = 0x29,
    /// No-op on the firmware side. Unused.
    #[allow(dead_code)]
    DataSentTcp = 0x2A,
    /// When called on a client socket, returns the number of bytes available
    /// for reading. On a server socket, allocates a new socket if a client is
    /// waiting and returns its socket number.
    ///
    /// Returns 255 if no client is waiting.
    AvailableDataTcp = 0x2B,
    /// Either peeks at or reads a single byte from a socket. Unused.
    #[allow(dead_code)]
    GetDataTcp = 0x2C,
    /// Starts a network connection on a socket.
    ///
    /// Though it’s called "TCP," it actually handles TCP, UDP, and multicast.
    StartClientTcp = 0x2D,
    StopClientTcp = 0x2E,
    GetClientStateTcp = 0x2F,

    // Disconnect = 0x30,
    // Unused = 0x31,
    // GetIdxRssi = 0x32,
    // GetIdxEnct = 0x33,
    /// Looks up the given host name to an IP address and returns 1 if it was
    /// found or 0 if it wasn’t.
    RequestHostByName = 0x34,
    /// Gets the most recent IP from a previously-successful RequestHostByName
    /// call.
    GetHostByName = 0x35,
    /// No-op on the firmware side. Unused.
    #[allow(dead_code)]
    StartScanNetworks = 0x36,
    GetFirmwareVersion = 0x37,
    // Unused = 0x38,
    // SendUdpData = 0x39,
    /// Returns the remote IP and port for a socket.
    // GetRemoteData = 0x3A,
    // GetTime = 0x3B,
    // GetIdxBssid = 0x3C,
    // GetIdxChannel = 0x3D,
    /// Pings a host by IP, with a given TTL
    Ping = 0x3E,
    /// Allocates a new socket number for use with StartClientTcp,
    /// StartServerTcp, &c.
    GetSocket = 0x3F,

    // SetClientCert = 0x40, // > 1.2.1
    // SetCertKey = 0x41, // > 1.2.1
    // Unused = 0x42,
    // Unused = 0x43,
    SendDataTcp = 0x44,
    GetDatabufTcp = 0x45,
    /// Writes data to a UDP socket
    // InsertDatabuf = 0x46,
    // SetEnterpriseIdent = 0x4A, // > 1.2.1
    // SetEnterpriseUsername = 0x4B, // > 1.2.1
    // SetEnterprisePassword = 0x4C, // > 1.2.1
    /// Not implemented in Adafruit firmware as of 1.6.1
    #[allow(dead_code)]
    SetEnterpriseCaCert = 0x4D, // > 1.2.1
    /// Not implemented in Adafruit firmware as of 1.6.1
    #[allow(dead_code)]
    SetEnterpriseCertKey = 0x4E, // > 1.2.1
    // SetEnterpriseEnable = 0x4F, // > 1.2.1
    /// Can be used to control the RGB LED on the AirLift FeatherWing.
    SetPinMode = 0x50,
    SetDigitalWrite = 0x51,
    SetAnalogWrite = 0x52,
    /// Available in Adafruit firmware v1.5.0 and above
    SetDigitalRead = 0x53,
    /// Available in Adafruit firmware v1.5.0 and above
    SetAnalogRead = 0x54,
    Start = 0xE0,
    End = 0xEE,
    Error = 0xEF,
}

impl Into<u8> for NinaCommand {
    fn into(self) -> u8 {
        self as u8
    }
}

#[repr(u8)]
enum NinaResponse {
    /// Response value sent when the command doesn’t really logically call for
    /// data to be sent back.
    Ack = 1,

    #[allow(dead_code)]
    /// Alternate response to Ack. We don’t check for this directly, we usually
    /// just error when we see that the expected response was not Ack.
    Error = 255,
}

impl Into<u8> for NinaResponse {
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
    /// As part of the chip’s response, it echoes back the
    /// [`NinaCommand`](enum.NinaCommand.html) byte, but with the high bit set
    /// to 1.
    const REPLY_FLAG: u8 = 1 << 7;

    /// Busy-waits for 100ms for the chip to start returning its response to our
    /// command.
    ///
    /// 100ms doesn’t seem like a lot, but by this point we’ve waited up to 10s
    /// for the chip to acknowledge being ready, which is the real waiting
    /// period for the chip to execute commands.
    ///
    /// Static method because `self` can’t be used when this is needed because
    /// `self.chip_select` gets mutably borrowed to create a
    /// [`SafeSpi`](struct.SafeSpi.html).
    fn wait_for_response_start<C, CT>(spi: &mut Spi, timer: &mut C) -> Result<(), Error<SpiError>>
    where
        C: embedded_hal::timer::CountDown<Time = CT>,
        CT: From<Duration>,
    {
        for _ in timer.timeout_iter(Duration::from_millis(100)) {
            let byte = spi.transfer_byte().map_err(Error::spi)?;

            if byte == NinaCommand::Start.into() {
                return Ok(());
            } else if byte == NinaCommand::Error.into() {
                return Err(Error::ErrorResponse);
            }
        }

        Err(Error::ResponseTimeout)
    }

    /// Ensures that the next byte from the WiFiNINI chip matches the expected
    /// character. Used to check that the reply command matches the given
    /// command, or that the Ack byte is sent.
    fn expect_byte(spi: &mut Spi, target_char: u8) -> Result<(), Error<SpiError>> {
        let v = spi.transfer_byte().map_err(Error::spi)?;

        if v == target_char {
            Ok(())
        } else {
            Err(Error::UnexpectedResponse(target_char, v))
        }
    }

    /// Sends a WiFiNINA command to the co-processor over SPI.
    fn send_command(
        &mut self,
        spi: &mut Spi,
        cmd: NinaCommand,
        params: Params<SendParam>,
    ) -> Result<(), Error<SpiError>> {
        let mut spi = self.chip_select.select(spi, &mut self.timer)?;

        let cmd_byte: u8 = cmd.into();
        // We keep track of the number of bytes sent so we can pad out to a
        // multiple of 4.
        let mut sent_len: usize = 0;

        let use_16_bit_length = params.use_16_bit_length();

        // The first part of the header is a "Start" token, the command we’re
        // sending, and then the number of parameters to the command.
        spi.write(&[
            NinaCommand::Start.into(),
            // Pedantic to mask out the top bit, since none of the commands use it.
            cmd_byte & !Self::REPLY_FLAG,
            params.len(),
        ])
        .map_err(Error::spi)?;

        // start + command + param count
        sent_len += 3;

        // Helper function to write a length value to the bus. Each parameter
        // for the command is prefixed by its length, either 1 byte or 2 bytes.
        // (Some commands explicitly use 2-byte lengths, for sending strings of
        // data.)
        let mut write_len = |spi: &mut Spi, len: usize| -> Result<(), Error<SpiError>> {
            sent_len += len;

            if use_16_bit_length {
                sent_len += 2;
                // length is in network byte order (not everything is)
                spi.write(&(len as u16).to_be_bytes()).map_err(Error::spi)?;
            } else {
                sent_len += 1;
                spi.write(&[len as u8]).map_err(Error::spi)?;
            };

            Ok(())
        };

        // Helper function to just write bytes to the bus.
        let write_bytes = |spi: &mut Spi, bytes: &mut dyn Iterator<Item = u8>| {
            spi.write_iter(bytes).map_err(Error::spi)
        };

        for p in params {
            match p {
                SendParam::Byte(b) => {
                    write_len(&mut spi, 1)?;
                    write_bytes(&mut spi, &mut [*b].iter().cloned())?;
                }

                SendParam::Word(w) => {
                    write_len(&mut spi, 2)?;
                    write_bytes(&mut spi, &mut w.to_be_bytes().iter().cloned())?;
                }

                SendParam::LEWord(w) => {
                    write_len(&mut spi, 2)?;
                    write_bytes(&mut spi, &mut w.to_le_bytes().iter().cloned())?;
                }

                SendParam::Bytes(it) => {
                    write_len(&mut spi, it.len())?;
                    write_bytes(&mut spi, it)?;
                }
            };
        }

        spi.write(&[NinaCommand::End.into()]).map_err(Error::spi)?;

        sent_len += 1;

        // Pad out request to a multiple of 4 bytes.
        while sent_len % 4 != 0 {
            spi.write(&[0]).map_err(Error::spi)?;
            sent_len += 1;
        }

        Ok(())
    }

    /// Accepts a response from the WiFiNINA chip. The params structure must be
    /// set up to match the expected response, as this method will copy values
    /// into it.
    fn receive_response(
        &mut self,
        spi: &mut Spi,
        cmd: NinaCommand,
        params: Params<RecvParam>,
    ) -> Result<(), Error<SpiError>> {
        let mut spi = self.chip_select.select(spi, &mut self.timer)?;

        let cmd_byte: u8 = cmd.into();
        Self::wait_for_response_start(&mut spi, &mut self.timer)?;
        // We expect that the server sends back the same command, with the high bit
        // set to indicate a reply.
        Self::expect_byte(&mut spi, Self::REPLY_FLAG | cmd_byte)?;

        let use_16_bit_length = params.use_16_bit_length();

        let read_len = |spi: &mut Spi, expect: Option<usize>| -> Result<usize, Error<SpiError>> {
            let len: usize;

            if use_16_bit_length {
                let bits = [
                    spi.transfer_byte().map_err(Error::spi)?,
                    spi.transfer_byte().map_err(Error::spi)?,
                ];

                len = u16::from_be_bytes(bits) as usize;
            } else {
                len = spi.transfer_byte().map_err(Error::spi)? as usize;
            };

            if let Some(expect) = expect {
                if len != expect {
                    return Err(Error::MismatchedParamSize(expect, len));
                }
            }

            Ok(len)
        };

        let param_count: u8 = spi.transfer_byte().map_err(Error::spi)?;
        let mut param_idx: u8 = 0;

        // We iterate through the provided parameters, filling them in from the
        // chip’s response stream.
        for param_handler in params {
            // This handles the case where the chip has told us, through
            // param_count, a number of response parameters that is fewer than
            // the number we’re prepared for. We’re ok with that as long as
            // every param from here to the end is an OptionalByte. Otherwise we
            // error.
            if param_idx == param_count {
                match param_handler {
                    RecvParam::OptionalByte(_) => continue,
                    _ => return Err(Error::MissingParam(param_idx)),
                }
            };

            match param_handler {
                RecvParam::Ack => {
                    read_len(&mut spi, Some(1))?;
                    Self::expect_byte(&mut spi, NinaResponse::Ack.into())?;
                }

                RecvParam::Byte(ref mut b) => {
                    read_len(&mut spi, Some(1))?;
                    **b = spi.transfer_byte().map_err(Error::spi)?;
                }

                RecvParam::OptionalByte(ref mut op) => {
                    read_len(&mut spi, Some(1))?;
                    op.replace(spi.transfer_byte().map_err(Error::spi)?);
                }

                RecvParam::Word(ref mut w) => {
                    read_len(&mut spi, Some(2))?;

                    let bits = [
                        spi.transfer_byte().map_err(Error::spi)?,
                        spi.transfer_byte().map_err(Error::spi)?,
                    ];

                    **w = u16::from_be_bytes(bits);
                }

                RecvParam::LEWord(ref mut w) => {
                    read_len(&mut spi, Some(2))?;

                    let bits = [
                        spi.transfer_byte().map_err(Error::spi)?,
                        spi.transfer_byte().map_err(Error::spi)?,
                    ];

                    **w = u16::from_le_bytes(bits);
                }

                RecvParam::Float(ref mut w) => {
                    read_len(&mut spi, Some(4))?;

                    let bits = [
                        spi.transfer_byte().map_err(Error::spi)?,
                        spi.transfer_byte().map_err(Error::spi)?,
                        spi.transfer_byte().map_err(Error::spi)?,
                        spi.transfer_byte().map_err(Error::spi)?,
                    ];

                    **w = f32::from_le_bytes(bits);
                }

                RecvParam::ByteArray(arr) => {
                    read_len(&mut spi, Some(arr.len()))?;

                    for i in 0..arr.len() {
                        arr[i] = spi.transfer_byte().map_err(Error::spi)?;
                    }
                }

                RecvParam::Buffer(arr, ref mut len) => {
                    let incoming_len = read_len(&mut spi, None)?;

                    // We’ll only read up to the buffer’s length.
                    **len = min(incoming_len, arr.len());

                    for i in 0..**len {
                        arr[i] = spi.transfer_byte().map_err(Error::spi)?;
                    }

                    // But we still have to pull the rest of the data off of the
                    // bus, we just ignore it.
                    for _ in **len..incoming_len {
                        spi.transfer_byte().map_err(Error::spi)?;
                    }
                }
            };

            param_idx += 1;
        }

        if param_count > param_idx {
            return Err(Error::UnexpectedParam(param_count));
        }

        Self::expect_byte(&mut spi, NinaCommand::End.into())?;

        Ok(())
    }

    /// Handles both sending and receiving of a single command.
    fn send_and_receive(
        &mut self,
        spi: &mut Spi,
        command: NinaCommand,
        send_params: Params<SendParam>,
        recv_params: Params<RecvParam>,
    ) -> Result<(), Error<SpiError>> {
        self.send_command(spi, command, send_params)?;
        self.receive_response(spi, command, recv_params)
    }
}

pub enum SendParam<'a> {
    /// Param is a single byte
    Byte(u8),
    /// Param is a word, to be sent in big-endian, network order
    Word(u16),
    /// Param is a word, to be sent in little-endian order. The WiFiNINA
    /// protocol differs command-to-command what byte order to use.
    LEWord(u16),
    /// Param is of arbitrary length (e.g. string data), though the length must
    /// be known so we can send it as a prefix
    Bytes(&'a mut dyn ExactSizeIterator<Item = u8>),
}

#[allow(dead_code)]
pub enum RecvParam<'a> {
    /// Asserts that the parameter will be the Ack byte (1)
    Ack,
    /// Receives a byte.
    Byte(&'a mut u8),
    /// Receives a byte, but does not error if the chip doesn’t provide it. Some
    /// commands don’t return a consistent number of values.
    OptionalByte(&'a mut Option<u8>),
    /// Receives a word in network byte order.
    Word(&'a mut u16),
    /// Receives a word in little-endian byte order, which is the native byte
    /// order on the ESP32.
    LEWord(&'a mut u16),
    /// Receives a 32-bit float.
    Float(&'a mut f32),
    /// Receives a known, fixed number of bytes (often an IP address).
    ByteArray(&'a mut [u8]),
    /// Reads bytes into a buffer, up to its length, and sets the second value
    /// to the number of bytes read. (Bytes beyond the buffer size are silently
    /// dropped.)
    Buffer(&'a mut [u8], &'a mut usize),
}

/// Structure to hold the set of parameters for both sending a command and
/// receiving a reply.
pub struct Params<'a, P> {
    /// Mutable for the sake of receiving values.
    params: &'a mut [P],
    /// Set to true if the length needs to be 16 bits instead of 8.
    use_16_bit_length: bool,
}

impl<'a, P> Params<'a, P> {
    pub fn none() -> Self {
        Params {
            params: &mut [],
            use_16_bit_length: false,
        }
    }

    pub fn of(params: &'a mut [P]) -> Self {
        Params {
            params,
            use_16_bit_length: false,
        }
    }

    pub fn with_16_bit_length(params: &'a mut [P]) -> Self {
        Params {
            params,
            use_16_bit_length: true,
        }
    }

    pub fn len(&self) -> u8 {
        self.params.len() as u8
    }

    pub fn use_16_bit_length(&self) -> bool {
        self.use_16_bit_length
    }
}

impl<'a, P> core::iter::IntoIterator for Params<'a, P> {
    type Item = &'a mut P;
    type IntoIter = core::slice::IterMut<'a, P>;

    fn into_iter(self) -> core::slice::IterMut<'a, P> {
        self.params.iter_mut()
    }
}
