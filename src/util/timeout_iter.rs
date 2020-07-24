use embedded_hal::timer::CountDown;

/// Converts a CountDown timer into an iterable object to do busy waits. Calling
/// next() returns a () value until the timer finishes. Once the timer is done,
/// the iterator is done and returns None.
///
/// Assumes that the timer will raise an nb::Error::WouldBlock before it has
/// gone off.
pub struct TimeoutIter<'a, C> {
    timer: &'a mut C,
}

impl<'a, C, CT> Iterator for TimeoutIter<'a, C>
where
    C: CountDown<Time = CT>,
{
    type Item = ();

    fn next(&mut self) -> Option<Self::Item> {
        match self.timer.wait() {
            Err(nb::Error::WouldBlock) => Some(()),
            Err(_) => panic!(),
            Ok(_) => None,
        }
    }
}

pub trait IntoTimeoutIter: CountDown
where
    Self: core::marker::Sized,
{
    fn timeout_iter<CT>(&mut self, timeout: CT) -> TimeoutIter<Self>
    where
        CT: Into<Self::Time>,
    {
        self.start(timeout);
        TimeoutIter { timer: self }
    }
}

impl<C: CountDown<Time = CT>, CT> IntoTimeoutIter for C {}
