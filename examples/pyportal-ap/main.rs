#![no_std]
#![no_main]

#[macro_use(block)]
extern crate nb;

#[allow(unused_imports)]
use panic_halt;

use core::fmt::Write;

use pyportal as hal;

use embedded_hal::digital::v2::{StatefulOutputPin, ToggleableOutputPin};
use hal::clock::GenericClockController;
use hal::entry;
use hal::pac::gclk;
use hal::pac::{CorePeripherals, Peripherals};
use hal::sercom;
use hal::{pins::Sets, Pins};

use wifinina::http::{HttpMethod, HttpRequestReader};
use wifinina::pyportal as pyportal_wifi;
use wifinina::{Error, Protocol};

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

    let Sets {
        d13,
        esp,
        spi,
        i2c,
        mut port,
        ..
    } = Pins::new(peripherals.PORT).split();

    let mut led = d13.into_push_pull_output(&mut port);

    let gclk2 = clocks
        .configure_gclk_divider_and_source(
            gclk::pchctrl::GEN_A::GCLK2,
            1,
            gclk::genctrl::SRC_A::DFLL,
            false,
        )
        .unwrap();

    let mut uart = helpers::stemma_uart(
        peripherals.SERCOM5,
        &mut clocks,
        &gclk2,
        &mut peripherals.MCLK,
        &mut port,
        i2c.scl,
        i2c.sda,
    );

    write!(&mut uart, "\r\n-- PYPORTAL START!! --\r\n").ok();

    let sys_tick = pyportal_wifi::sys_tick(core_peripherals.SYST, &mut clocks);

    write!(&mut uart, "Making SPI…\r\n").ok();

    let mut spi = pyportal_wifi::spi(
        &mut clocks,
        peripherals.SERCOM2,
        &mut peripherals.MCLK,
        &mut port,
        spi,
    );

    let (mut wifi, ..) = pyportal_wifi::wifi(&mut port, esp, &spi, &sys_tick).unwrap();

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

    write!(&mut uart, "Creating 'PyPortal' access point…\r\n").ok();
    wifi.wifi_create_ap(&mut spi, "PyPortal", None, 0).ok();

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
        handle_client(&mut uart, client_socket, &mut led);
    }
}

fn handle_client<
    // These traits are implemented by ConnectedSocket, and they’re all we need
    // to handle this request / response. (The actual ConnectedSocket type is
    // unfortunately a mess of generics to use.)
    S: genio::Read<ReadError = nb::Error<Error<sercom::Error>>> + core::fmt::Write,
    P: StatefulOutputPin + ToggleableOutputPin,
>(
    uart: &mut dyn core::fmt::Write,
    client_socket: S,
    led: &mut P,
) {
    let mut request_reader = HttpRequestReader::from_read(client_socket);

    match block!(request_reader.read_head()) {
        Ok(head) => {
            write!(uart, "{} {}\r\n", head.method, head.path).ok();

            if head.path == "/" {
                match head.method {
                    HttpMethod::Get => handle_page(&mut request_reader.free(), led),
                    HttpMethod::Post => {
                        // We get a POST when the user presses the
                        // toggle button.
                        led.toggle().ok();
                        handle_redirect(&mut request_reader.free(), "/");
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

fn handle_page<W: core::fmt::Write, P: StatefulOutputPin>(writer: &mut W, pin: &mut P) {
    write!(writer, "HTTP/1.1 200 OK\r\n").ok();
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
                </style>
            </head>
            <body>
                <h1>Hello, World!</h1>
                The LED is <strong>{}</strong>.

                <form method=POST style='margin-top: 20px; text-align: center'>
                    <button type=submit style='
                        font-weight:bold;
                        -webkit-appearance: none;
                        border: 1px solid #444;
                        background: #eee;
                        padding: 15px;
                        display: inline-block;'>
                        Toggle LED
                    </button>
                </form>
            </body>
        </html>",
        if pin.is_set_high().ok().unwrap() {
            "on"
        } else {
            "off"
        }
    )
    .ok();
}

fn handle_redirect<W: core::fmt::Write>(writer: &mut W, location: &str) {
    write!(writer, "HTTP/1.1 303 See Other\r\n").ok();
    write!(writer, "Location: {}\r\n", location).ok();
}

fn handle_not_found<W: core::fmt::Write>(writer: &mut W) {
    write!(writer, "HTTP/1.1 404 Not Found\r\n").ok();
}

fn handle_method_not_allowed<W: core::fmt::Write>(writer: &mut W) {
    write!(writer, "HTTP/1.1 405 Method Not Allowed\r\n").ok();
}
