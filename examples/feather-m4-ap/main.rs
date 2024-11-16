//! Example for a Feather M4 Express with an AirLift FeatherWing.
//!
#![no_std]
#![no_main]

#[macro_use(block)]
extern crate nb;

#[allow(unused_imports)]
use panic_halt;

use core::fmt::Write;

use feather_m4 as hal;

use hal::clock::GenericClockController;
use hal::entry;
use hal::pac::{CorePeripherals, Peripherals};
use hal::prelude::*;
use hal::Pins;

use wifinina::feather_m4 as feather_m4_wifi;
use wifinina::Protocol;

use weehttp::{HttpMethod, HttpRequestReader, UrlEncodedParams};

use heapless;
use heapless::consts::*;

#[path = "../helpers.rs"]
mod helpers;

#[entry]
fn main() -> ! {
    let mut peripherals = Peripherals::take().unwrap();
    let core_peripherals = CorePeripherals::take().unwrap();

    let mut clocks = GenericClockController::with_internal_32kosc(
        peripherals.GCLK,
        &mut peripherals.MCLK,
        &mut peripherals.OSC32KCTRL,
        &mut peripherals.OSCCTRL,
        &mut peripherals.NVMCTRL,
    );

    let mut pins = Pins::new(peripherals.PORT);

    let mut uart = hal::uart(
        &mut clocks,
        115200u32.hz(),
        peripherals.SERCOM5,
        &mut peripherals.MCLK,
        // NOTE: As of v0.5.0, these are wrong. pins has the wrong types.
        pins.d1,
        pins.d0,
        &mut pins.port,
    );

    write!(&mut uart, "\r\n-- AIRLIFT START!! --\r\n").ok();

    let sys_tick = feather_m4_wifi::sys_tick(core_peripherals.SYST, &mut clocks);

    write!(&mut uart, "Making SPI…\r\n").ok();

    let mut spi = feather_m4_wifi::spi(
        &mut clocks,
        peripherals.SERCOM1,
        &mut peripherals.MCLK,
        &mut pins.port,
        pins.sck,
        pins.mosi,
        pins.miso,
    );

    let (mut wifi, ..) = feather_m4_wifi::wifi(
        &mut pins.port,
        pins.d11,
        pins.d12,
        pins.d13,
        &spi,
        &sys_tick,
    )
    .unwrap();

    write!(&mut uart, "Checking firmware version…\r\n").ok();

    match wifi.get_firmware_version(&mut spi) {
        Ok((buf, len)) => {
            write!(
                &mut uart,
                "WiFiNINA Version {}\r\n",
                // -1 to remove the null character
                core::str::from_utf8(&buf[..len - 1]).unwrap()
            )
            .ok();
        }
        Err(err) => {
            write!(&mut uart, "Could not get WiFiNINA version: {:?}\r\n", err).ok();
        }
    }

    write!(&mut uart, "Creating 'AirLift' access point…\r\n").ok();
    wifi.wifi_create_ap(&mut spi, "AirLift", None, 0).ok();

    let server_socket = wifi
        .server_start(&mut spi, Protocol::Tcp, 80, None)
        .unwrap();

    let network_info = wifi.network_info(&mut spi).unwrap();

    write!(
        &mut uart,
        "Server listening: http://{}.{}.{}.{}/\r\n",
        network_info.ip[0], network_info.ip[1], network_info.ip[2], network_info.ip[3]
    )
    .ok();

    loop {
        let client_socket = block!(wifi.server_select(&mut spi, &server_socket)).unwrap();
        let socket = client_socket.suspend();
        handle_client(&mut uart, &mut wifi, &mut spi, socket);
    }
}

fn handle_client(
    uart: &mut dyn core::fmt::Write,
    wifi: &mut feather_m4_wifi::WifiNina,
    spi: &mut feather_m4_wifi::Spi,
    socket: feather_m4_wifi::Socket,
) {
    let mut color: Option<[u8; 3]> = None;

    // Create a scope that wifi and spi are borrowed in
    {
        let client_socket = wifi.socket_resume(spi, socket);
        let mut request_reader = HttpRequestReader::from_read(client_socket);

        match block!(request_reader.read_head()) {
            Ok(head) => {
                write!(uart, "{} {}\r\n", head.method, head.path).ok();

                if head.path == "/" {
                    match head.method {
                        HttpMethod::Get => handle_page(&mut request_reader.free()),
                        HttpMethod::Post => {
                            let mut params =
                                UrlEncodedParams::<_, _, U1024, U255, U255>::from_reader(
                                    request_reader,
                                );

                            while let Some((name, value)) = block!(params.next()).unwrap() {
                                match name.as_str() {
                                    "color" => match value.as_str() {
                                        "teal" => {
                                            color = Some([0, 255, 200]);
                                        }
                                        "yellow" => {
                                            color = Some([255, 200, 0]);
                                        }
                                        "magenta" => {
                                            color = Some([200, 0, 255]);
                                        }
                                        "off" => {
                                            color = Some([0, 0, 0]);
                                        }
                                        _ => {}
                                    },
                                    _ => {}
                                }
                            }

                            // We get a POST when the user presses the
                            // toggle button.
                            handle_redirect(&mut params.free().free(), "/");
                        }
                        _ => {
                            handle_method_not_allowed(&mut request_reader.free());
                        }
                    }
                } else {
                    handle_not_found(&mut request_reader.free());
                }
            }
            Err(err) => {
                write!(uart, "Error parsing HTTP head. :( {:?} \r\n", err).ok();
            }
        }
    }

    // This is down here since we can’t send other WifiNina commands while a
    // ConnectedSocket is around.
    if let Some(c) = color {
        feather_m4_wifi::set_airlift_rgb(wifi, spi, c).ok();
    }
}

fn handle_page<W: core::fmt::Write>(writer: &mut W) {
    write!(writer, "HTTP/1.0 200 OK\r\n").ok();
    write!(writer, "Content-type: text/html\r\n").ok();
    write!(writer, "\r\n").ok();
    write!(
        writer,
        "
        <!DOCTYPE>
        <html>
            <head>
                <meta name='viewport' content='width=device-width, initial-scale=1'/>
                <style type='text/css'>
                body {{
                    font-family: sans-serif;
                }}

                button {{
                    font-weight:bold;
                    -webkit-appearance: none;
                    border: 1px solid #444;
                    background: #eee;
                    padding: 15px;
                    display: inline-block;
                }}
                </style>
            </head>
            <body>
                <h1>Hello, World!</h1>

                <form method=POST style='margin-top: 20px; text-align: center'>
                    <button type=submit name='color' value='teal'>
                        Teal
                    </button>
                    <button type=submit name='color' value='yellow'>
                        Yellow
                    </button>
                    <button type=submit name='color' value='magenta'>
                        Magenta
                    </button>
                    <button type=submit name='color' value='off'>
                        Off
                    </button>
                </form>
            </body>
        </html>",
    )
    .ok();
}

fn handle_redirect<W: core::fmt::Write>(writer: &mut W, location: &str) {
    write!(writer, "HTTP/1.0 303 See Other\r\n").ok();
    write!(writer, "Location: {}\r\n", location).ok();
}

fn handle_not_found<W: core::fmt::Write>(writer: &mut W) {
    write!(writer, "HTTP/1.0 404 Not Found\r\n").ok();
}

fn handle_method_not_allowed<W: core::fmt::Write>(writer: &mut W) {
    write!(writer, "HTTP/1.0 405 Method Not Allowed\r\n").ok();
}
