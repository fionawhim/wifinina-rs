/// Trait for an object that represents a chip select pin selecting a member of
/// the bus.
///
/// It is expected that a ChipSelect implementation is guarding a bus. If
/// selection succeeds, it returns a SafeSpi instance that can be used to access
/// the bus. Once SafeSpi goes out of scope, it will call deselect.
pub trait ChipSelect {
    type Spi;

    /// Disables the chip selection.
    fn deselect(&mut self);
}

/// A wrapper around an Spi bus and ChipSelect instance that will deselect the
/// ChipSelect when the wrapper goes out-of-scope.
pub struct SafeSpi<'a, S, CS>
where
    CS: ChipSelect<Spi = S>,
{
    cs: &'a mut CS,
    spi: &'a mut S,
}

impl<'a, S, CS> SafeSpi<'a, S, CS>
where
    CS: ChipSelect<Spi = S>,
{
    pub fn new(spi: &'a mut S, cs: &'a mut CS) -> Self {
        SafeSpi { spi, cs }
    }
}

impl<'a, S, CS: ChipSelect<Spi = S>> Drop for SafeSpi<'a, S, CS> {
    fn drop(&mut self) {
        self.cs.deselect();
    }
}

impl<'a, S, CS: ChipSelect<Spi = S>> core::ops::Deref for SafeSpi<'a, S, CS> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.spi
    }
}

impl<'a, S, CS: ChipSelect<Spi = S>> core::ops::DerefMut for SafeSpi<'a, S, CS> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.spi
    }
}
