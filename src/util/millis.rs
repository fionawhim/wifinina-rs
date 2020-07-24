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

impl Milliseconds {
    // Implementation derived from https://playground.arduino.cc/Code/TimingRollover/
    pub fn is_after<T>(self, other: T) -> bool
    where
        T: Into<Milliseconds>,
    {
        // Need to convert to i32 before subtracting to avoid underflow panic
        other.into().0 as i32 - self.0 as i32 >= 0
    }
}

impl core::ops::Add for Milliseconds {
    type Output = Milliseconds;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}
