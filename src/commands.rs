pub mod network;
pub mod socket;
pub mod wifi;

use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::spi::FullDuplex;

use crate::util::millis::{Milliseconds, U32Ext};
use crate::util::spi_ext::SpiExt;
use crate::util::timeout_iter::IntoTimeoutIter;

use crate::{Error, WifiNina};

#[derive(Debug, Copy, Clone)]
#[repr(u8)]
#[allow(dead_code)]
pub enum NinaCommand {
    SetNetwork = 0x10,
    SetNetworkAndPassphrase = 0x11,
    SetKey = 0x12,
    // Test = 0x13,
    SetIpConfig = 0x14,
    SetDnsConfig = 0x15,
    SetHostname = 0x16,
    SetPowerMode = 0x17,
    SetApNetwork = 0x18,
    SetApPassphrase = 0x19,
    SetDebug = 0x1A,

    GetConnectionStatus = 0x20,
    GetIpAddress = 0x21,
    GetMacAddress = 0x22,
    GetCurrentSsid = 0x23,
    GetCurrentRssi = 0x25,
    GetCurrentEnct = 0x26,
    ScanNetworks = 0x27,

    GetSocket = 0x3F,
    GetStateTcp = 0x29,
    DataSentTcp = 0x2A,
    AvailableDataTcp = 0x2B,
    GetDataTcp = 0x2C,

    StartClientTcp = 0x2D,
    StopClientTcp = 0x2E,
    GetClientStateTcp = 0x2F,

    Disconnect = 0x30,
    GetIdxRssi = 0x32,
    GetIdxEnct = 0x33,

    RequestHostByName = 0x34,
    GetHostByName = 0x35,
    StartScanNetworks = 0x36,
    GetFirmwareVersion = 0x37,
    Ping = 0x3E,

    SendDataTcp = 0x44,
    GetDatabufTcp = 0x45,

    SetEnterpriseIdent = 0x4A,
    SetEnterpriseUsername = 0x4B,
    SetEnterprisePassword = 0x4C,
    SetEnterpriseEnable = 0x4F,

    SetPinMode = 0x50,
    SetDigitalWrite = 0x51,
    SetAnalogWrite = 0x52,

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
    Ack = 1,

    #[allow(dead_code)]
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
    // Pin that the co-processor uses to say if it’s ready.
    BusyPin: InputPin,
    // Pin we use to tell the chip to listen on the SPI bus.
    CsPin: OutputPin,
    Spi: FullDuplex<u8, Error = SpiError>
        + embedded_hal::blocking::spi::Write<u8, Error = SpiError>
        + embedded_hal::blocking::spi::WriteIter<u8, Error = SpiError>,
    CountDown: embedded_hal::timer::CountDown<Time = CountDownTime>,
    CountDownTime: From<Milliseconds>,
{
    /// Flag added to response commands from the chip. It echoes the command
    /// that was sent, but with this bit set to 1.
    const REPLY_FLAG: u8 = 1 << 7;

    // Static method because it needs to be called while chip_select is mutably
    // borrowed
    fn wait_for_response_start<C, CT>(spi: &mut Spi, timer: &mut C) -> Result<(), Error<SpiError>>
    where
        C: embedded_hal::timer::CountDown<Time = CT>,
        CT: From<Milliseconds>,
    {
        for _ in timer.timeout_iter(100.ms()) {
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

    /// Sends a WiFiNINA command to the co-processor.
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

        sent_len += 3;

        // Helper function to write a length value to the bus. Each parameter
        // for the command is prefixed by its length, either 1 byte or 2 bytes.
        // (Some commands explicitly use 2-byte lengths, for sending strings of
        // data.)
        let mut write_len = |spi: &mut Spi, len: usize| -> Result<(), Error<SpiError>> {
            sent_len += len;

            if use_16_bit_length {
                sent_len += 2;
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

            return Ok(len);
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

                RecvParam::ExpectByte(b) => {
                    read_len(&mut spi, Some(1))?;
                    Self::expect_byte(&mut spi, *b)?;
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

                RecvParam::ByteArray(arr) => {
                    read_len(&mut spi, Some(arr.len()))?;

                    for i in 0..arr.len() {
                        arr[i] = spi.transfer_byte().map_err(Error::spi)?;
                    }
                }

                RecvParam::Buffer(arr, ref mut len) => {
                    **len = read_len(&mut spi, None)?;

                    for i in 0..**len {
                        arr[i] = spi.transfer_byte().map_err(Error::spi)?;
                    }
                }
            };

            param_idx += 1;
        }

        if param_count > param_idx {
            return Err(Error::UnexpectedParam(param_count));
        }

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

    /// Puts the chip in debug mode. This causes it to write diagnostic info to
    /// its UART.
    pub fn set_debug(&mut self, spi: &mut Spi, enabled: bool) -> Result<(), Error<SpiError>> {
        self.send_and_receive(
            spi,
            NinaCommand::SetDebug,
            Params::of(&mut [SendParam::Byte(enabled as u8)]),
            Params::of(&mut [RecvParam::Ack]),
        )
    }
}

pub enum SendParam<'a> {
    /// Param is a single byte
    Byte(u8),
    /// Param is a word, to be sent in big-endian, network order
    Word(u16),
    /// Param is a word, to be sent in little-endian order
    LEWord(u16),
    /// Param is of arbitrary length (e.g. string data), though the length must
    /// be known so we can send it as a prefix
    Bytes(&'a mut dyn ExactSizeIterator<Item = u8>),
}

#[allow(dead_code)]
pub enum RecvParam<'a> {
    Ack,
    Byte(&'a mut u8),
    OptionalByte(&'a mut Option<u8>),
    ExpectByte(u8),
    Word(&'a mut u16),
    LEWord(&'a mut u16),
    ByteArray(&'a mut [u8]),
    Buffer(&'a mut [u8], &'a mut usize),
}

/// Structure to hold the set of parameters for both sending a command and
/// receiving a reply.
///
/// TODO(fiona): Should this be separate between Send and Receive, since the
/// former doesn’t need to be mutable? We might need to duplicate the impls,
/// though.
pub struct Params<'a, P> {
    /// Mutable for the sake of receiving values, since the library won’t
    /// allocate its own memory for them.
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
        self.params.into_iter()
    }
}
