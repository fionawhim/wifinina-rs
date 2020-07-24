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
use hal::sercom;
use hal::sercom::PadPin;
use hal::time::Hertz;
use hal::{pins::Sets, Pins};

use cortex_m_rt::exception;

use wifinina::WifiNina;

mod systick_delay;

#[exception]
fn SysTick() {
    systick_delay::SysTickDelay::interrupt();
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
        esp_uart,
        spi,
        i2c,
        mut port,
        ..
    } = Pins::new(peripherals.PORT).split();

    let mut led = d13.into_push_pull_output(&mut port);

    clocks.configure_gclk_divider_and_source(
        gclk::pchctrl::GEN_A::GCLK2,
        1,
        gclk::genctrl::SRC_A::DFLL,
        false,
    );

    led.set_high().unwrap();

    let gclk2 = clocks
        .get_gclk(gclk::pchctrl::GEN_A::GCLK2)
        .expect("Could not get clock 2");

    let tx: sercom::Sercom5Pad1<_> = i2c.scl.into_pull_down_input(&mut port).into_pad(&mut port);
    let rx: sercom::Sercom5Pad0<_> = i2c.sda.into_pull_down_input(&mut port).into_pad(&mut port);

    let uart_clk = clocks
        .sercom5_core(&gclk2)
        .expect("Could not configure sercom5 clock");

    let mut uart = sercom::UART5::new(
        &uart_clk,
        Hertz(19200),
        peripherals.SERCOM5,
        &mut peripherals.MCLK,
        (tx, rx),
    );

    write!(&mut uart, "\r\n-- PYPORTAL START --\r\n");
    // block!(uart.flush());

    // clocks.configure_gclk_divider_and_source(
    //     gclk::pchctrl::GEN_A::GCLK3,
    //     1,
    //     gclk::genctrl::SRC_A::DFLL,
    //     false,
    // );
    // let gclk3 = clocks
    //     .get_gclk(gclk::pchctrl::GEN_A::GCLK3)
    //     .expect("Could not get clock 3");

    // We swap rx and tx here because the ESP32’s output is our input.
    let esp_tx: sercom::Sercom4Pad1<_> = esp_uart
        .rx
        .into_pull_down_input(&mut port)
        .into_pad(&mut port);

    let esp_rx: sercom::Sercom4Pad0<_> = esp_uart
        .tx
        .into_pull_down_input(&mut port)
        .into_pad(&mut port);

    write!(&mut uart, "Starting ESP32 UART…\r\n");

    let esp_uart_clk = clocks
        .sercom4_core(&gclk2)
        .expect("Could not configure sercom4 clock");

    let mut esp_uart = sercom::UART4::new(
        &esp_uart_clk,
        Hertz(19200),
        peripherals.SERCOM4,
        &mut peripherals.MCLK,
        (esp_tx, esp_rx),
    );

    write!(&mut uart, "Starting SPI…\r\n");

    let mut spi = hal::spi_master(
        &mut clocks,
        8000000.hz(),
        peripherals.SERCOM2,
        &mut peripherals.MCLK,
        spi.sck,
        spi.mosi,
        spi.miso,
        &mut port,
    );

    let sysclock_hertz: Hertz = clocks.gclk0().into();
    write!(&mut uart, "sysclock running at {}hz\r\n", sysclock_hertz.0);

    let mut sys_tick = systick_delay::SysTickDelay::new(core_peripherals.SYST, sysclock_hertz.0);

    let esp_cs = esp.cs.into_push_pull_output(&mut port);
    let esp_busy = esp.busy.into_floating_input(&mut port);
    let mut esp_reset = esp.reset.into_push_pull_output(&mut port);

    write!(&mut uart, "Starting WiFiNINA…\r\n");

    let mut wifi = WifiNina::new(
        &spi,
        esp_cs,
        esp_busy,
        &mut esp_reset,
        sys_tick.count_down(),
    )
    .unwrap();

    wifi.set_debug(&mut spi, true).unwrap();

    wifi.wifi_create_ap(&mut spi, "WiFiNINA", 0).unwrap();
    // led.set_low().unwrap();

    loop {
        led.set_low().unwrap();
        sys_tick.delay_ms(200);
        led.set_high().unwrap();
        sys_tick.delay_ms(200);

        /*
        match esp_uart.read() {
            Ok(byte) => {
                block!(uart.write(byte)).unwrap();

                // Blink the red led to show that a character has arrived
                led.set_high().unwrap();
                sys_tick.delay_ms(2);
                led.set_low().unwrap();
            }
            Err(_) => sys_tick.delay_ms(5),
        };*/
    }
}
