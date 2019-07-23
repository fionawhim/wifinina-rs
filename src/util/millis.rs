#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Milliseconds(pub u32);

pub trait U32Ext {
    fn ms(self) -> Milliseconds;
}

impl U32Ext for u32 {
    fn ms(self) -> Milliseconds {
        Milliseconds(self)
    }
}

impl core::ops::Add for Milliseconds {
    type Output = Milliseconds;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}
