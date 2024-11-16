use core::cmp::min;
use core::convert::TryInto;
use core::time::Duration;

use embedded_hal::digital::v2::{InputPin, OutputPin};
use embedded_hal::spi::FullDuplex;

use heapless::consts::*;
use heapless::String;
use nb::block;

use crate::commands::*;
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

impl Into<u8> for WifiStatus {
    fn into(self) -> u8 {
        self as u8
    }
}

#[repr(u8)]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum WifiEncryptionType {
    /// Unencrypted
    Open = 7,
    /// WEP
    Wep = 5,
    /// WPA PSK
    WpaPsk = 2,
    /// WPA2 PSK or WPA/WPA2 PSK
    Wpa2Psk = 4,

    /// Unknown
    Unknown = 255,
}

impl From<u8> for WifiEncryptionType {
    fn from(s: u8) -> Self {
        match s {
            2 => WifiEncryptionType::WpaPsk,
            4 => WifiEncryptionType::Wpa2Psk,
            5 => WifiEncryptionType::Wep,
            7 => WifiEncryptionType::Open,

            _ => WifiEncryptionType::Unknown,
        }
    }
}

impl Into<u8> for WifiEncryptionType {
    fn into(self) -> u8 {
        self as u8
    }
}

#[repr(u8)]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum WifiPowerMode {
    /// Corresponds to the ESP32’s `WIFI_PS_NONE`.
    Full = 0,
    /// Corresponds to the ESP32’s `WIFI_PS_MODEM`.
    Low = 1,
}

impl Into<u8> for WifiPowerMode {
    fn into(self) -> u8 {
        self as u8
    }
}

pub struct WifiScanResult {
    ssid: heapless::String<U32>,
    bssid: [u8; 6],
    rssi: i8,
    channel: u8,
    encryption_type: WifiEncryptionType,
}

/// Result struct for scanning for SSIDs.
///
/// Because the WiFiNINA chip has a fixed maximum of 10 networks, we can just
/// allocate all the memory we’d need for this rather than try and stream it
/// out.
pub struct WifiScanResults {
    /// max number of returned SSIDs is 10. See MAX_SCAN_RESULTS in WiFi.h
    ///
    /// Tuple is the length of the buffer that's filled, followed by the buffer
    /// itself.
    pub ssids: [(usize, [u8; 255]); 10],
    /// Number of ssids buffers that have data read into them.
    pub ssids_count: usize,
}

impl Default for WifiScanResults {
    fn default() -> Self {
        WifiScanResults {
            ssids: [(0, [0; 255]); 10],
            ssids_count: 0,
        }
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
    /// Returns the current WifiStatus for the chip.
    pub fn wifi_status(&mut self, spi: &mut Spi) -> Result<WifiStatus, Error<SpiError>> {
        let mut status: u8 = 255;

        self.send_and_receive(
            spi,
            NinaCommand::GetConnectionStatus,
            Params::none(),
            Params::of(&mut [RecvParam::U8(&mut status)]),
        )?;

        Ok(status.into())
    }

    /// Joins a WiFi network.
    ///
    /// Waits up to 15 seconds for the connection to succeed.
    pub fn wifi_connect(
        &mut self,
        spi: &mut Spi,
        ssid: &str,
        password: Option<&str>,
    ) -> Result<WifiStatus, Error<SpiError>> {
        if let Some(password) = password {
            self.send_and_receive(
                spi,
                NinaCommand::SetNetworkAndPassphrase,
                Params::of(&mut [
                    SendParam::U8Array(ssid.as_bytes()),
                    SendParam::U8Array(password.as_bytes()),
                ]),
                Params::of(&mut [RecvParam::Ack]),
            )?;
        } else {
            self.send_and_receive(
                spi,
                NinaCommand::SetNetwork,
                Params::of(&mut [SendParam::U8Array(ssid.as_bytes())]),
                Params::of(&mut [RecvParam::Ack]),
            )?;
        }

        let mut last_status = WifiStatus::UnknownStatus;

        // Wait 15 seconds for the Wifi to stabilize.
        for _ in 0..15 {
            last_status = self.wifi_status(spi)?;

            if last_status == WifiStatus::Connected {
                return Ok(last_status);
            } else if last_status == WifiStatus::ConnectFailed
                || last_status == WifiStatus::ConnectionLost
                || last_status == WifiStatus::Disconnected
            {
                break;
            }

            self.timer.start(Duration::from_millis(1_000));
            block!(self.timer.wait()).ok();
        }

        Err(Error::ConnectionFailed(last_status))
    }

    /// Starts an access point with the provided name, password, and 802.11b
    /// channel.
    ///
    /// Waits up to 15 seconds for the access point to be created.
    pub fn wifi_create_ap(
        &mut self,
        spi: &mut Spi,
        name: &str,
        password: Option<&str>,
        channel: u8,
    ) -> Result<WifiStatus, Error<SpiError>> {
        if let Some(password) = password {
            self.send_and_receive(
                spi,
                NinaCommand::SetApPassphrase,
                Params::of(&mut [
                    SendParam::U8Array(name.as_bytes()),
                    SendParam::U8Array(password.as_bytes()),
                    SendParam::U8(channel),
                ]),
                Params::of(&mut [RecvParam::Ack]),
            )?;
        } else {
            self.send_and_receive(
                spi,
                NinaCommand::SetApNetwork,
                Params::of(&mut [SendParam::U8Array(name.as_bytes()), SendParam::U8(channel)]),
                Params::of(&mut [RecvParam::Ack]),
            )?;
        }

        let mut last_status = WifiStatus::UnknownStatus;

        // Wait 15 seconds for the Wifi to stabilize.
        for _ in 0..15 {
            last_status = self.wifi_status(spi)?;

            if last_status == WifiStatus::ApListening || last_status == WifiStatus::ApConnected {
                return Ok(last_status);
            } else if last_status == WifiStatus::ApFailed {
                break;
            }

            self.timer.start(Duration::from_millis(1_000));
            block!(self.timer.wait()).ok();
        }

        Err(Error::ConnectionFailed(last_status))
    }

    /// Fills in a [`WifiScanResults`](struct.WifiScanResults.html) with the
    /// available networks.
    ///
    /// Note that this disconnects any current Wi-Fi connection and access
    /// point.
    ///
    /// Returns the number of networks filled in in the `WifiScanResults`.
    pub fn wifi_scan<'a>(
        &'a mut self,
        spi: &'a mut Spi,
        result: &mut WifiScanResults,
    ) -> Result<u8, Error<SpiError>> {
        self.send_command(spi, NinaCommand::ScanNetworks, Params::none())?;

        // This next stuff is all taken from receive_response, unrolled because
        // we need to handle a variable response.
        let mut spi = self.chip_select.select(spi, &mut self.timer)?;

        let cmd_byte: u8 = NinaCommand::ScanNetworks.into();
        Self::wait_for_response_start(&mut spi, &mut self.timer)?;
        Self::expect_byte(&mut spi, Self::REPLY_FLAG | cmd_byte)?;

        let ssids_count = spi.transfer_byte().map_err(Error::spi)?;
        result.ssids_count = ssids_count.into();

        for i in 0..min(ssids_count.into(), result.ssids.len()) {
            let buf = &mut result.ssids[i].1;

            let ssid_len = spi.transfer_byte().map_err(Error::spi)?;

            // We’ll only read up to the buffer’s length.
            let read_len: usize = min(ssid_len.into(), buf.len());

            for i in 0..read_len {
                (*buf)[i] = spi.transfer_byte().map_err(Error::spi)?;
            }

            // But we still have to pull the rest of the data off of the
            // bus.
            for _ in read_len..ssid_len.into() {
                spi.transfer_byte().map_err(Error::spi)?;
            }

            result.ssids[i].0 = read_len;
        }

        Self::expect_byte(&mut spi, NinaCommand::End.into())?;

        Ok(ssids_count)
    }

    pub fn wifi_set_power_mode(
        &mut self,
        spi: &mut Spi,
        mode: WifiPowerMode,
    ) -> Result<(), Error<SpiError>> {
        self.send_and_receive(
            spi,
            NinaCommand::SetPowerMode,
            Params::of(&mut [SendParam::U8(mode.into())]),
            Params::of(&mut [RecvParam::Ack]),
        )?;

        Ok(())
    }

    /// Gets the current SSID and saves it into the provided byte buffer.
    ///
    /// Returns the size of the SSID.
    pub fn wifi_ssid(&mut self, spi: &mut Spi, ssid: &mut [u8]) -> Result<usize, Error<SpiError>> {
        let mut name_len = 0usize;

        self.send_and_receive(
            spi,
            NinaCommand::GetCurrentSsid,
            Params::none(),
            Params::of(&mut [RecvParam::U8Buffer(ssid, &mut name_len)]),
        )?;

        Ok(name_len)
    }

    /// Gets the current BSSID.
    pub fn wifi_bssid(&mut self, spi: &mut Spi) -> Result<[u8; 6], Error<SpiError>> {
        let mut bssid = [0u8; 6];

        self.send_and_receive(
            spi,
            NinaCommand::GetCurrentBssid,
            Params::none(),
            Params::of(&mut [RecvParam::U8Array(&mut bssid)]),
        )?;

        Ok(bssid)
    }

    /// Gets the current signal strength.
    pub fn wifi_rssi(&mut self, spi: &mut Spi) -> Result<i8, Error<SpiError>> {
        let mut rssi = 0i32;

        self.send_and_receive(
            spi,
            NinaCommand::GetCurrentRssi,
            Params::none(),
            Params::of(&mut [RecvParam::I32LE(&mut rssi)]),
        )?;

        Ok(rssi.try_into().unwrap_or(0))
    }

    /// Gets the encryption type for the current Wi-Fi network.
    pub fn wifi_encryption_type(
        &mut self,
        spi: &mut Spi,
    ) -> Result<WifiEncryptionType, Error<SpiError>> {
        let mut enc_type = 0u8;

        self.send_and_receive(
            spi,
            NinaCommand::GetCurrentEnct,
            Params::none(),
            Params::of(&mut [RecvParam::U8(&mut enc_type)]),
        )?;

        Ok(WifiEncryptionType::from(enc_type))
    }
}
