#![no_std]
#![no_main]

#[macro_use(block)]
extern crate nb;

#[allow(unused_imports)]
use panic_halt;

use core::fmt::Write;

use pyportal as hal;

use hal::clock::GenericClockController;
use hal::entry;
use hal::pac::gclk;
use hal::pac::{CorePeripherals, Peripherals};
use hal::prelude::*;
use hal::{pins::Sets, Pins};

use wifinina::http::{HttpMethod, HttpRequestReader, HttpResponseReader};
use wifinina::pyportal as pyportal_wifi;
use wifinina::pyportal::prelude::*;
use wifinina::{Destination, Protocol, WifiScanResults, WifiStatus};

use genio::Read;

use heapless;
use heapless::consts::*;

use serde::Deserialize;
use serde_json_core;

use smart_leds::{SmartLedsWrite, RGB8};

#[path = "../helpers.rs"]
mod helpers;
use helpers::{HtmlEscape, UriDecode};

type Color = [u8; 3];

#[derive(Deserialize, Debug, Copy, Clone)]
struct Colors {
    result: [Option<Color>; 5],
}

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
        neopixel,
        mut port,
        ..
    } = Pins::new(peripherals.PORT).split();

    // Not used, but we keep it for debugging.
    let _led = d13.into_push_pull_output(&mut port);

    let mut neopixel = helpers::onboard_neopixel(&mut port, neopixel);

    let gclk2 = clocks
        .configure_gclk_divider_and_source(
            gclk::pchctrl::GEN_A::GCLK2,
            1,
            gclk::genctrl::SRC_A::DFLL,
            false,
        )
        .unwrap();

    let mut uart = helpers::stemma_uart(
        &mut clocks,
        peripherals.SERCOM5,
        &gclk2,
        &mut peripherals.MCLK,
        &mut port,
        i2c.scl,
        i2c.sda,
    );

    write!(&mut uart, "\r\n-- PYPORTAL START!! --\r\n").ok();

    let sys_tick = pyportal_wifi::sys_tick(core_peripherals.SYST, &mut clocks);

    let mut spi = pyportal_wifi::spi(
        peripherals.SERCOM2,
        &mut clocks,
        &mut peripherals.MCLK,
        &mut port,
        spi,
    );

    let (mut wifi, ..) = pyportal_wifi::wifi(&mut port, esp, &spi, &sys_tick).unwrap();

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

    // Create the server_socket once outside of the loop because once it’s
    // created it doesn’t ever get collected.
    let server_socket = wifi
        .server_start(&mut spi, Protocol::Tcp, 80, None)
        .unwrap();

    // Initialize these to avoid having to use the web page each time.
    let mut ssid: Option<heapless::String<U256>> = Some(heapless::String::from("GrogNet"));
    let mut password: Option<heapless::String<U32>> = Some(heapless::String::from("fuffle"));

    // Loop between two modes: getting the SSID and password, and doing network
    // stuff.
    loop {
        if ssid == None || password == None {
            let mut scan_results: WifiScanResults = WifiScanResults::default();

            write!(&mut uart, "Network scan…\r\n").ok();

            // Cyan while scanning
            neopixel.write([[0, 20, 30]].iter().cloned()).ok();
            wifi.wifi_scan(&mut spi, &mut scan_results).ok();
            write!(&mut uart, "{} networks found\r\n", scan_results.ssids_count).ok();

            write!(&mut uart, "Creating 'PyPortal' access point…\r\n").ok();

            // Blue when we’re in AP mode needing someone to connect to give the
            // password.
            neopixel.write([[0, 0, 30]].iter().cloned()).ok();
            wifi.wifi_create_ap(&mut spi, "PyPortal", None, 5).ok();

            let network_info = wifi.network_info(&mut spi).unwrap();
            write!(
                &mut uart,
                "Server listening: http://{}.{}.{}.{}/\r\n",
                network_info.ip[0], network_info.ip[1], network_info.ip[2], network_info.ip[3]
            )
            .ok();

            // // Loop to handle server mode requests
            while ssid == None || password == None {
                let mut client_socket =
                    block!(wifi.server_select(&mut spi, &server_socket)).unwrap();

                handle_client(
                    &mut uart,
                    &mut client_socket,
                    &scan_results,
                    &mut ssid,
                    &mut password,
                );
            }
        }

        write!(
            &mut uart,
            "Connecting to '{}' access point…\r\n",
            ssid.as_ref().unwrap(),
        )
        .ok();

        // Yellow while we’re waiting for the wifi to connect
        neopixel.write([[30, 30, 0]].iter().cloned()).ok();

        match wifi.wifi_connect(
            &mut spi,
            ssid.as_ref().unwrap(),
            Some(password.as_ref().unwrap()),
        ) {
            Ok(WifiStatus::Connected) => {
                write!(&mut uart, "…success!\r\n",).ok();
            }
            Ok(status) => {
                write!(&mut uart, "…failed: {:?}\r\n", status).ok();
                // Setting to none so we fail back and restart the loop with
                // setting the access point.
                password = None;
                continue;
            }
            Err(err) => {
                write!(&mut uart, "SPI Error: {:?}\r\n", err).ok();
                // Setting to none so we fail back and restart the loop with
                // setting the access point.
                password = None;
                continue;
            }
        }

        // Switch to green once we’re connected.
        neopixel.write([[0, 30, 0]].iter().cloned()).ok();

        loop {
            let colors = fetch_colors(&mut uart, &mut wifi, &mut spi).unwrap();

            if let Some(colors) = colors {
                for _ in 0..10 {
                    for color in colors.result.iter() {
                        if let Some(color) = color {
                            neopixel
                                .write(
                                    [RGB8 {
                                        // We have to scale these down because
                                        // full brightness NeoPixel is tooo
                                        // bright.
                                        r: color[0] / 20,
                                        g: color[1] / 20,
                                        b: color[2] / 20,
                                    }]
                                    .iter()
                                    .cloned(),
                                )
                                .ok();
                        }

                        sys_tick.delay().delay_ms(3_000);
                    }
                }
            } else {
                sys_tick.delay().delay_ms(30_000);
            }
        }
    }
}

fn handle_client(
    uart: &mut dyn core::fmt::Write,
    client_socket: &mut pyportal_wifi::ConnectedSocket,
    scan_results: &WifiScanResults,
    ssid: &mut Option<heapless::String<U256>>,
    password: &mut Option<heapless::String<U32>>,
) {
    let mut request_reader = HttpRequestReader::from_read(client_socket);

    match block!(request_reader.read_head()) {
        Ok(head) => {
            write!(uart, "{} {}\r\n", head.method, head.path).ok();

            if head.path == "/" {
                match head.method {
                    HttpMethod::Get => handle_home_page(&mut request_reader.free(), scan_results),
                    HttpMethod::Post => {
                        handle_connect_post(&mut request_reader, ssid, password);
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

fn handle_home_page<W: core::fmt::Write>(writer: &mut W, scan_results: &WifiScanResults) {
    write!(writer, "HTTP/1.1 200 OK\r\n").ok();
    write!(writer, "Content-Type: text/html; charset=utf-8\r\n").ok();
    write!(writer, "\r\n").ok();
    write!(
        writer,
        "
        <!DOCTYPE>
        <html>
            <head>
                <meta name='viewport' content='width=device-width, initial-scale=1'/>
                <title>PyPortal Connect</title>
                <style type='text/css'>
                body {{
                    font-family: sans-serif;
                }}

                form {{
                    max-width: 400px;
                    margin-top: 20px;
                }}

                .form-row {{
                    display: flex;
                    align-items: center;
                    margin: 8px 0;
                }}

                .form-row label {{
                    font-weight: bold;
                    width: 30%;
                    margin-right: 1em;
                    text-align: right;
                }}

                .form-row input, .form-row select {{
                    display: inline-block;
                    flex-grow: 1;
                }}

                .button-row {{
                    text-align: center;
                    margin-top: 20px;
                }}

                .button-row button {{
                    display: inline-block;
                    -webkit-appearance: none;
                    padding: 10px 20px;
                    border: 2px solid #dde0dd;
                    border-radius: 1px;
                    font-weight: bold;
                    cursor: pointer;
                }}
                </style>

                <script type=\"text/javascript\">
                function selectChange(val) {{
                    var otherRow = document.getElementById('other-row');
                    if (val == '') {{
                        otherRow.style.display = '';
                    }} else {{
                        otherRow.style.display = 'none';
                    }}
                }}
                </script>
            </head>
            <body>
                <h1>Hello, World!</h1>

                <form method=\"POST\" action=\"/\" >
                <div class=\"form-row\">
                    <label for='ssid'>Network:</label>
                    <select id='ssid' name='ssid' onChange=\"selectChange(this.value)\">
        "
    )
    .ok();

    for i in 0..scan_results.ssids_count {
        let ssid =
            core::str::from_utf8(&scan_results.ssids[i].1[0..scan_results.ssids[i].0]).unwrap();

        write!(
            writer,
            "<option value=\"{}\">{}</option>\r\n",
            HtmlEscape::from_str(ssid),
            HtmlEscape::from_str(ssid)
        )
        .ok();
    }

    write!(
        writer,
        "
                     <option disabled>-----------------</option>
                     <option value=\"\">Other…</option>
                    </select>
                </div>

                <div class=\"form-row\" id=\"other-row\" style=\"display: none;\">
                    <label for=\"other\">SSID:</label>
                    <input id=\"other\" type=\"text\" name=\"other\" />
                </div>

                <div class=\"form-row\">
                    <label for=\"password\">Password:</label>
                    <input id=\"password\" type=\"password\" name=\"password\" />
                </div>

                <div class=\"button-row\">
                    <button type=\"submit\">Connect</button>
                </div>
                </form>
            </body>
        </html>",
    )
    .ok();
}

fn handle_connect_post<R: genio::Read>(
    reader: &mut R,
    ssid: &mut Option<heapless::String<U256>>,
    password: &mut Option<heapless::String<U32>>,
) {
    // Buffer big enough to get the HTTP form post body
    let mut req = heapless::String::<U1024>::new();

    let mut buf = [0u8; 255];
    while let Ok(len) = reader.read(&mut buf) {
        req.push_str(core::str::from_utf8(&buf[..len]).unwrap())
            .ok();
    }

    // The POST body is URL-encoded, so we split on &.
    for param in req.split('&') {
        let mut part_split = param.split('=');

        let key = part_split.next();
        let val = part_split.next();

        if key == None || val == None {
            continue;
        }

        let decoded_val = UriDecode::from_str(val.unwrap());

        match key {
            // "ssid" is the select box, "other" is the input box
            Some("ssid") | Some("other") => {
                if val.unwrap().len() > 0 {
                    ssid.replace(heapless::String::new());
                    write!(ssid.as_mut().unwrap(), "{}", decoded_val).ok();
                }
            }
            Some("password") => {
                password.replace(heapless::String::new());
                write!(password.as_mut().unwrap(), "{}", decoded_val).ok();
            }
            _ => {}
        }
    }
}

// impl<L> genio::ExtendFromReader for heapless::String<L> {}

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

fn fetch_colors<W: core::fmt::Write>(
    uart: &mut W,
    wifi: &mut pyportal_wifi::WifiNina,
    spi: &mut pyportal_wifi::Spi,
) -> Result<Option<Colors>, pyportal_wifi::Error> {
    write!(uart, "Connecting to colormind.io…\r\n").ok();

    let mut color_socket = wifi.connect(
        spi,
        Protocol::Tcp,
        Destination::Hostname("colormind.io"),
        80,
    )?;

    let req = "{\"model\":\"default\"}";

    write!(uart, "Connection success\r\n").ok();

    // TODO(fiona): It would be nice to find a library to handle creating these
    // HTTP requests.
    write!(&mut color_socket, "POST /api/ HTTP/1.1\r\n")?;
    write!(&mut color_socket, "Host: colormind.io\r\n")?;
    write!(&mut color_socket, "User-Agent: PyPortal\r\n")?;
    write!(&mut color_socket, "Accept: */*\r\n")?;
    write!(&mut color_socket, "Content-Length: {}\r\n", req.len())?;
    write!(
        &mut color_socket,
        "Content-Type: application/x-www-form-urlencoded\r\n"
    )?;

    write!(&mut color_socket, "\r\n")?;
    write!(&mut color_socket, "{}", req)?;

    let mut response_reader = HttpResponseReader::from_read(color_socket);
    let head = block!(response_reader.read_head()).unwrap();

    if head.code != 200 {
        write!(uart, "Did not get 200 response: {}", head.code).ok();
        return Ok(None);
    }

    let mut resp = heapless::String::<U1024>::new();
    let mut buf = [0u8; 255];
    loop {
        match block!(response_reader.read(&mut buf)) {
            Ok(0) => break,
            Ok(len) => {
                resp.push_str(core::str::from_utf8(&buf[..len]).unwrap())
                    .ok();
            }
            Err(err) => {
                write!(uart, "Error reading body: {:?}", err).ok();
            }
        }
    }

    write!(uart, "Response: {}\r\n", resp).ok();

    Ok(Some(serde_json_core::from_str::<Colors>(&resp).unwrap()))
}
