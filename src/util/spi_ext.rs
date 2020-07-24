use embedded_hal::spi::FullDuplex;
use nb::block;

pub trait SpiExt: FullDuplex<u8> {
    /// Pumps the SPI bus for the next byte by sending a 0 byte and receiving
    /// the response, since all SPI communication is full-duplex.
    fn transfer_byte(&mut self) -> Result<u8, Self::Error> {
        block!(self.send(0u8)).and_then(|_| block!(self.read()))
    }
}

impl<S: FullDuplex<u8>> SpiExt for S {}
