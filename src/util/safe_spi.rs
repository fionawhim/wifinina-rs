pub trait ChipSelect {
    type Spi;

    fn deselect(&mut self);
}

// A wrapper around an Spi bus that will deselect its ChipSelect when it
// goes out-of-scope.
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
